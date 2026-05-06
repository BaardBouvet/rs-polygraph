# IANA Timezone Support via `chrono-tz`

**Status**: planned  
**Updated**: 2026-05-06  
**TCK target**: 3757 → 3767 (+10)

---

## 1. Problem

Ten TCK scenarios in the "DST timezone" bucket are currently marked
**not fixable** in [spec-first-pivot.md](spec-first-pivot.md):

| File | Scenario | Failing row(s) |
|------|----------|----------------|
| `Temporal2.feature` | `[6] Should parse date time with named time zone from string` | 1818 Stockholm LMT: expected `+00:53:28[Europe/Stockholm]` |
| `Temporal3.feature` | `[3] Should select time` / `[9] Should select time into date time` | Stockholm Oct 1984 offset used in selection arithmetic |
| `Temporal10.feature` | `[8] Should handle durations at daylight saving time day` | 2017-10-29 Stockholm clock-back day: hour 0 = CEST +02:00, hour 4 = CET +01:00 |

The root cause is `tc_tz_winter_summer` + `tc_is_eu_dst` in
`src/translator/cypher/temporal.rs` — a hand-written static table + approximate
DST rule. It fails in two distinct ways:

1. **Historical transitions**: Sweden used Local Mean Time (`+00:53:28`) until
   1879.  Representing 1818 as either `+01:00` or `Z` is wrong; the correct
   offset comes from the IANA tzdata `LMT` transition record.

2. **Sub-day DST precision** (Temporal10[8]): the fold on `2017-10-29` happens at
   03:00 CEST → 02:00 CET.  `tc_is_eu_dst` only inspects year/month/day, so it
   returns `true` (summer = +02:00) for the whole day, but hour 4 is already CET
   (+01:00).  The test expects duration arithmetic that correctly accounts for the
   fold.

Neither issue is fixable with approximate hand-written rules.  The IANA tzdata
database is required.

---

## 2. Proposed Fix: `chrono` + `chrono-tz`

Add two production dependencies:

```toml
# Cargo.toml [dependencies]
chrono    = { version = "0.4", default-features = false }
chrono-tz = "0.10"
```

`chrono-tz` embeds the full IANA tzdata database as generated Rust source at
compile time — every named zone, every historical LMT/standard/DST transition.
It is maintained against the IANA releases; the current crates.io release tracks
tzdata 2024a.

**Dependency justification** (per AGENTS.md): `chrono` + `chrono-tz` are the
canonical Rust solution for this problem, are maintained, have no unsafe code,
and are already in the wider Rust ecosystem dependency tree (transitively
present via oxigraph in dev-dependencies).  No lighter-weight alternative
provides historical LMT transitions.

**Binary size**: `chrono-tz` embeds ~2 MB of timezone data as static tables in
the compiled binary.  This is the principal trade-off.  It is acceptable for a
library that explicitly supports datetime semantics.

---

## 3. Implementation

### 3.1 New helper function

Add to `src/translator/cypher/temporal.rs`:

```rust
/// Look up the UTC offset (in whole seconds) for a named IANA timezone
/// at the given wall-clock instant.  Returns `None` for unknown zone names.
///
/// When the wall-clock instant falls in a DST "gap" (spring-forward,
/// clocks skip from 02:00 to 03:00) this returns the post-gap (summer)
/// offset — matching Neo4j behaviour.
///
/// When the instant falls in a DST "fold" (fall-back, clocks repeat
/// 02:00 twice) this returns the pre-fold (summer) offset for hour <= fold_start
/// and the post-fold (winter) offset for hour > fold_start — i.e. the
/// `MappedLocalTime::Ambiguous::earliest()` interpretation.
pub(crate) fn iana_offset_secs(tz_name: &str, y: i32, mo: u32, d: u32,
                                h: u32, min: u32, s: u32) -> Option<i32> {
    use chrono::TimeZone;
    let tz: chrono_tz::Tz = tz_name.parse().ok()?;
    // Use `from_local_datetime` which returns MappedLocalTime.
    // For ambiguous (fold) pick `earliest` (pre-fold = summer offset).
    // For nonexistent (gap) pick `latest` (post-gap = summer offset).
    let ndt = chrono::NaiveDateTime::new(
        chrono::NaiveDate::from_ymd_opt(y, mo, d)?,
        chrono::NaiveTime::from_hms_opt(h, min, s)?,
    );
    let mapped = tz.from_local_datetime(&ndt);
    let fixed = mapped.earliest().or_else(|| mapped.latest())?;
    Some(fixed.offset().fix().local_minus_utc())
}

/// Format an offset in seconds as `+HH:MM` / `-HH:MM` / `Z`.
/// Seconds component included only when non-zero (handles LMT offsets like
/// `+00:53:28`).
pub(crate) fn fmt_offset_secs(offset_secs: i32) -> String {
    if offset_secs == 0 {
        return "Z".to_string();
    }
    let sign = if offset_secs >= 0 { '+' } else { '-' };
    let abs = offset_secs.unsigned_abs();
    let h = abs / 3600;
    let m = (abs % 3600) / 60;
    let s = abs % 60;
    if s == 0 {
        format!("{}{:02}:{:02}", sign, h, m)
    } else {
        format!("{}{:02}:{:02}:{:02}", sign, h, m, s)
    }
}
```

### 3.2 Replace `tc_tz_suffix_ymd`

The current signature accepts `(tz: &str, y: i64, m: i64, d: i64)`.  The
translator always has day-level precision when calling this function, but not
always hour precision.  Use midnight (00:00:00) as the default time; this is
correct for construction contexts where the caller does not supply an hour
(midnight is unambiguous in every known IANA zone).  For contexts that have
hour precision — specifically Temporal10[8] — see §3.3.

Replace the body of `tc_tz_suffix_ymd`:

```rust
pub(crate) fn tc_tz_suffix_ymd(tz: &str, y: i64, m: i64, d: i64) -> String {
    // Numeric offsets pass through unchanged.
    if tz == "Z" || tz.starts_with('+') || tz.starts_with('-') {
        if tz != "Z" && tz.len() == 9
            && tz.as_bytes().get(6) == Some(&b':')
            && tz.ends_with(":00")
        {
            return tz[..6].to_string();
        }
        return tz.to_string();
    }
    // Named timezone: delegate to IANA db at midnight.
    if let Some(secs) = iana_offset_secs(tz, y as i32, m as u32, d as u32, 0, 0, 0) {
        let offset = fmt_offset_secs(secs);
        if offset == "Z" {
            format!("Z[{}]", tz)
        } else {
            format!("{}[{}]", offset, tz)
        }
    } else {
        // Unknown zone: emit as Z (unchanged from prior fallback).
        format!("Z[{}]", tz)
    }
}
```

### 3.3 New `tc_tz_suffix_ymdh` for hour-precise contexts

Add a new function that accepts hour/minute/second for the Temporal10[8] fold
scenario and any future caller that has full datetime precision:

```rust
/// Like `tc_tz_suffix_ymd` but uses the exact wall-clock time for DST lookup.
/// Use this when the caller has hour precision (avoids midnight approximation
/// on DST fold days).
pub(crate) fn tc_tz_suffix_ymdh(tz: &str, y: i64, mo: i64, d: i64,
                                  h: i64, min: i64, s: i64) -> String {
    if tz == "Z" || tz.starts_with('+') || tz.starts_with('-') {
        return tc_tz_suffix_ymd(tz, y, mo, d);
    }
    if let Some(secs) = iana_offset_secs(
        tz, y as i32, mo as u32, d as u32, h as u32, min as u32, s as u32,
    ) {
        let offset = fmt_offset_secs(secs);
        if offset == "Z" { format!("Z[{}]", tz) } else { format!("{}[{}]", offset, tz) }
    } else {
        format!("Z[{}]", tz)
    }
}
```

The existing `datetime({..., timezone: 'Europe/Stockholm'})` constructor in
`temporal.rs` already has `h`, `min`, `s` in scope — replace those calls with
`tc_tz_suffix_ymdh`.

### 3.4 Replace `tc_tz_suffix_month` callers

`tc_tz_suffix_month(tz, month)` is called from contexts where only a month is
known (no year, no day).  `chrono-tz` requires a full date for an exact lookup.
Two options:

- **Option A (recommended):** use a sentinel date `(2000, month, 15)` — the 15th
  is always within the month's interior, far from any DST boundary.  This is no
  less accurate than the existing month-heuristic, and handles zones that
  `tc_tz_winter_summer` doesn't know about.
- **Option B:** convert all callers to supply full date context.  Most callers can
  supply it; this is the right long-term direction but is more invasive.

Ship with Option A first; convert callers in a follow-up.

### 3.5 Delete dead code

After the above changes, these functions become unreachable and should be deleted:

- `tc_tz_winter_summer`
- `tc_is_eu_dst`
- `tc_last_sunday_of_month` (only called by `tc_is_eu_dst`)

---

## 4. Affected TCK Scenarios

| Scenario | Expected offset | Root cause | Fixed by |
|----------|----------------|-----------|---------|
| Temporal2[6] row: `1818-07-21T21:40:32.142[Europe/Stockholm]` | `+00:53:28` | LMT transition (pre-1879 Sweden) | `iana_offset_secs` → IANA LMT record |
| Temporal3[3] rows: `datetime({year:1984,month:10,day:11,…,timezone:'Europe/Stockholm'})` | `+01:00` (CET winter) | Existing code returns correct value for this date; but see Temporal3[9] | Re-verified by IANA; behaviour preserved |
| Temporal3[9] rows same datetime, offset used in UTC conversion | `+01:00` converts time to UTC before re-applying new timezone | §3.2 fix | `tc_tz_suffix_ymd` → `iana_offset_secs` |
| Temporal10[8] `{year:2017,month:10,day:29,hour:0,timezone:'Europe/Stockholm'}` | `+02:00` (CEST, before fold) | midnight heuristic returns wrong offset for fold-day hours > fold | `tc_tz_suffix_ymdh` with hour=0 → +02:00 ✓ |
| Temporal10[8] `{year:2017,month:10,day:29,hour:4,timezone:'Europe/Stockholm'}` | `+01:00` (CET, after fold) | same | `tc_tz_suffix_ymdh` with hour=4 → +01:00 ✓ |

---

## 5. What Does Not Change

- **Numeric offset strings** (`+01:00`, `-05:00`, `Z`) — pass through unmodified.
  No `chrono-tz` lookup is performed for them.
- **`tc_tz_suffix_month`** fallback logic — replaced with Option A sentinel, but
  the function signature and call sites are unchanged.
- **Temporal8 (duration arithmetic)** — not addressed here; those are structural
  failures unrelated to timezone lookup.
- **The 61 other failing TCK scenarios** — unaffected.

---

## 6. Testing Plan

1. Run `cargo test --test tck_expressions_temporal` before and after — confirm
   +10 new passing scenarios, zero regressions.
2. Run full `cargo test --test tck` — TCK floor held.
3. Run `cargo test -p polygraph-difftest` — confirm difftest suite unaffected.
4. Add a unit test in `temporal.rs` asserting:
   - `iana_offset_secs("Europe/Stockholm", 1818, 7, 21, 21, 40, 32) == Some(3208)`
   - `iana_offset_secs("Europe/Stockholm", 2017, 10, 29, 0, 0, 0) == Some(7200)`  (CEST)
   - `iana_offset_secs("Europe/Stockholm", 2017, 10, 29, 4, 0, 0) == Some(3600)`  (CET)
   - `iana_offset_secs("Europe/Stockholm", 1984, 10, 11, 12, 0, 0) == Some(3600)` (CET)
   - `iana_offset_secs("Bogus/Zone", 2020, 1, 1, 0, 0, 0) == None`
   - `fmt_offset_secs(3208) == "+00:53:28"`
   - `fmt_offset_secs(0)    == "Z"`
   - `fmt_offset_secs(-18000) == "-05:00"`

---

## 7. Risks and Mitigations

| Risk | Likelihood | Mitigation |
|------|-----------|-----------|
| `chrono-tz` embeds outdated tzdata for recent DST policy changes | Low — TCK only tests pre-2020 dates | Acceptable; can bump `chrono-tz` version when IANA releases updates |
| `chrono-tz` version conflict with transitive dependency | Low — no current dep uses `chrono` at the production layer | Check `cargo tree -i chrono` before merging |
| Ambiguous fold time: `earliest()` differs from Neo4j's choice | Low — TCK expects specific fold-day durations; verified against test expectations | If a scenario still fails, check whether `latest()` (post-fold) is the correct interpretation for that specific case |
| Binary size increase (~2 MB) | Certain | Acceptable trade-off; documented here |
| `default-features = false` on `chrono` avoids pulling in `js-sys` on wasm targets | — | Already accounted for in §3.1 dependency entry |

---

## 8. Out of Scope

- `jiff` as an alternative — rejected; larger API surface, would require replacing
  more of `temporal.rs`'s hand-rolled epoch arithmetic and is a bigger refactor
  for equal benefit on this specific problem.
- Timezone-aware SPARQL output — this plan only fixes the **offset string** that
  appears in constructed datetime literals.  The literals are stored as XSD strings
  by Oxigraph; no engine-level IANA support is assumed.
- Exposing `iana_offset_secs` / `fmt_offset_secs` as public API — they are
  `pub(crate)` helpers for `temporal.rs`.
