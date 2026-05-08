# Temporal Code Cleanup — chrono/chrono-tz Follow-up

**Status**: planned  
**Updated**: 2026-05-08  
**Depends on**: `chrono` + `chrono-tz` production dependencies (landed in v0.10.0)

---

## 1. Background

`iana-timezone.md` described adding `chrono` and `chrono-tz` to replace the
hand-written static DST table in `temporal.rs`. Those dependencies have been
added and the primary IANA lookup path (`iana_offset_secs`, `fmt_offset_secs`,
`tc_tz_suffix_ymdh`) is implemented. However, the old fallback code was left in
place for safety during the initial landing.

This plan describes the follow-up cleanup now that `chrono` is a confirmed
dependency. `temporal.rs` is currently ~4000 lines. The goal is to delete all
code that is made redundant by `chrono`, and separately to refactor the
hand-rolled calendar arithmetic that chrono also provides.

---

## 2. Phase 1 — Delete the static DST table (low risk, ~70 lines)

These three functions are the original hand-written timezone implementation.
They still exist as the "fallback" inside `tc_tz_suffix_month` and
`tc_tz_suffix_ymdh`, but `iana_offset_secs` now covers every zone they handled
(and many more).

### 2.1 Functions to delete

| Function | Lines | Why it is safe to delete |
|----------|-------|--------------------------|
| `tc_tz_winter_summer()` | ~22 | Hardcoded offsets for ~15 zones; superseded by IANA db |
| `tc_is_eu_dst_h()` | ~38 | Approximate last-Sunday rule; superseded by IANA db |
| `tc_last_sunday_of_month()` | ~9 | Helper only for `tc_is_eu_dst_h` |

### 2.2 Callers to fix

**`tc_tz_suffix_ymdh()`** currently calls `iana_offset_secs` first and falls
back to `tc_tz_winter_summer` + `tc_is_eu_dst_h`. The fallback branch must be
replaced with a simple `format!("Z[{}]", tz)` for unrecognised zone names
(same as the current unknown-zone behaviour):

```rust
pub(crate) fn tc_tz_suffix_ymdh(tz: &str, y: i64, m: i64, d: i64, h: i64) -> String {
    if tz == "Z" || tz.starts_with('+') || tz.starts_with('-') {
        // ... numeric passthrough unchanged ...
    }
    if let Some(secs) = iana_offset_secs(tz, y as i32, m as u32, d as u32, h as u32, 0, 0) {
        let offset = fmt_offset_secs(secs);
        return if offset == "Z" { format!("Z[{}]", tz) } else { format!("{}[{}]", offset, tz) };
    }
    // Unknown zone (no IANA entry): emit bare Z suffix.
    format!("Z[{}]", tz)
}
```

**`tc_tz_suffix_month()`** currently uses `tc_tz_winter_summer` to guess an
offset from month alone. Replace with a sentinel date lookup — year 2000, day
15 (well inside any month, far from DST boundaries):

```rust
fn tc_tz_suffix_month(tz: &str, month: i64) -> String {
    if tz == "Z" || tz.starts_with('+') || tz.starts_with('-') {
        return tz.to_string();
    }
    let secs = iana_offset_secs(tz, 2000, month as u32, 15, 0, 0, 0);
    let offset = secs.map(fmt_offset_secs).unwrap_or_else(|| "Z".to_string());
    if offset == "Z" { format!("Z[{}]", tz) } else { format!("{}[{}]", offset, tz) }
}
```

### 2.3 Verification

Run the full TCK suite and confirm no temporal scenarios regress. The DST tests
that were failing before `chrono-tz` should still pass (they go through
`tc_tz_suffix_ymdh`, not `tc_tz_suffix_month`).

**Estimated savings**: ~70 lines deleted, zero new lines.

---

## 3. Phase 2 — Replace hand-rolled calendar arithmetic with chrono (medium risk, ~150 lines)

`temporal.rs` contains a proleptic Gregorian calendar implementation — epoch
counting, leap-year detection, month-length tables, ISO week calculation — all
written from scratch. `chrono::NaiveDate` provides all of these as a maintained,
tested API. Replace them one by one.

### 3.1 Candidates and their chrono equivalents

| Custom function | Lines | chrono replacement |
|-----------------|-------|--------------------|
| `temporal_is_leap(y)` | 3 | `NaiveDate::from_ymd_opt(y, 2, 29).is_some()` |
| `temporal_dim(y, m)` | 12 | `NaiveDate::from_ymd_opt(y, m+1, 1).unwrap_or(NaiveDate::from_ymd_opt(y+1,1,1)).pred_opt()` or `NaiveDate::from_ymd_opt(y, m, 1).unwrap().days_in_month()` (feature-gated; easier via `pred_opt`) |
| `temporal_epoch(y, m, d)` | 10 | `NaiveDate::from_ymd_opt(y, m, d)?.num_days_from_ce()` |
| `temporal_from_epoch(n)` | 30 | `NaiveDate::from_num_days_from_ce_opt(n)` → `(d.year(), d.month(), d.day())` |
| `temporal_week_to_date(iso_year, week, dow)` | 10 | `NaiveDate::from_isoywd_opt(iso_year, week, Weekday::Mon + dow - 1)` |
| `temporal_ordinal_to_md(y, ord)` | 20 | `NaiveDate::from_yo_opt(y, ord)` → `(d.month(), d.day())` |
| `date_to_iso_week(y, m, d)` | 14 | `nd.iso_week().year()`, `.iso_week().week()`, `nd.weekday().num_days_from_monday() + 1` |
| `tc_iso_week_year(y, m, d)` | 7 | `NaiveDate::from_ymd_opt(y,m,d)?.iso_week().year()` |

Note: chrono uses `i32` for years; `temporal.rs` uses `i64`. Casting is safe for
all plausible calendar dates (±292 years from epoch for i32). Add a saturating
cast `y as i32` at the boundary. `temporal_epoch` uses a different epoch origin
than `num_days_from_ce`; note that `chrono`'s CE epoch is Jan 1, year 1 = day
1, which matches the existing `temporal_epoch` convention. Confirm with a unit
test before removing `temporal_epoch`.

### 3.2 What to keep

`temporal_quarter_to_md` has no chrono equivalent (chrono has no quarter
concept). Keep as-is.

The remaining `temporal_*` functions (`temporal_frac`, `temporal_sub_second`,
`temporal_get_i/f/s`, `temporal_date_from_map`, etc.) are specific to
openCypher map-constructor semantics and have no chrono equivalents. Keep them.

### 3.3 Sequencing

Do this as a series of small commits, replacing one function at a time and
running `cargo test` after each. The call graph is deep (`temporal_epoch` has
~16 call sites) so do it last after replacing `temporal_from_epoch`,
`temporal_week_to_date`, etc. that currently depend on it.

**Estimated savings**: ~110–150 lines deleted; ~50–60 new chrono calls in their
place.

---

## 4. Phase 3 — Minor remaining cleanup (optional, low priority)

### 4.1 `parse_tz_offset_s()`

Lines ~1189–1211. Hand-parses `+HH:MM` / `+HH:MM:SS` offset strings into a
total-seconds integer. Could use `chrono::FixedOffset` parsing but the
conversion is not straightforward. Leave unless it becomes a bug source.

### 4.2 `tc_tz_suffix()` rename

`tc_tz_suffix(tz)` is a thin wrapper that calls `tc_tz_suffix_month(tz, 1)`.
After Phase 1 rewrites `tc_tz_suffix_month`, consider inlining it and removing
the indirection.

---

## 5. TCK exit criterion

After all phases: TCK pass rate must not decrease from baseline. Run:

```sh
cargo test --test tck
cargo test --test tck_expressions_temporal
```

Expected: same pass count as before each phase (cleanup is behaviour-neutral
for Phases 1 and 2).

---

## 6. Estimated total reduction

| Phase | Lines deleted | Lines added | Net |
|-------|--------------|-------------|-----|
| 1 — DST table | ~70 | ~15 | −55 |
| 2 — Calendar arithmetic | ~150 | ~60 | −90 |
| 3 — Minor cleanup | ~20 | ~0 | −20 |
| **Total** | **~240** | **~75** | **−165** |

`temporal.rs` would shrink from ~4000 to ~3800 lines. This is worthwhile mainly
for maintainability (no more subtle DST or leap-year edge cases hiding in custom
code) rather than raw line-count reduction.
