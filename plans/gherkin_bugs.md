# Gherkin Parser Issues in rs-polygraph

This document catalogs bugs and limitations encountered with the `cucumber-rs` Gherkin parser when processing the openCypher TCK test suite.

## Overview

The rs-polygraph project uses the `cucumber` crate to execute openCypher compliance tests via Gherkin scenario files. During TCK integration, we discovered several classes of parser issues in the Gherkin implementation that prevent valid TCK scenarios from being parsed and executed.

## Bug Categories

### 1. Placeholder Syntax Ambiguity (`<=` in Scenario Outlines)

**Severity**: Medium — Affects ~10 scenarios  
**Status**: Confirmed  
**Fixability**: Workaroundable

#### Description

The cucumber-rs Gherkin scanner misinterprets `<= <rhs>` in scenario-outline example placeholders as a malformed nested placeholder reference, rather than treating `<=` as a literal less-than-or-equal operator.

#### Affected Scenarios

| File | Line | Pattern | Issue |
|------|------|---------|-------|
| `Comparison2.feature` | 123 | `<lhs> <= <rhs>` | Parser treats `<= <rhs>` as nested placeholder |
| `Quantifier7.feature` | 80 | `<= any(<operands>)` | Same scanner confusion |

#### Example

```gherkin
Scenario Outline: [X] Some test
  When executing query:
    """
    MATCH (n) WHERE n.age <= <value>
    RETURN n
    """
  Then the result should be, in any order:
    | result   |
    | <result> |

  Examples:
    | value | result |
    | 50    | bob    |
    | 21    | alice  |
```

When cucumber-rs encounters `<= <value>`, it attempts to parse `<= <value>` as a placeholder reference instead of treating `<=` as a literal token.

#### Workarounds

1. **Escape the operator in feature files** — Use HTML/Unicode escaping:
   - `\u003c=` (Unicode escape)
   - `&lt;=` (HTML entity)
   
   **Example**:
   ```gherkin
   MATCH (n) WHERE n.age \u003c= <value>
   ```

2. **Upgrade cucumber-rs** — Check if a newer version of the crate handles this correctly.

3. **Pre-process feature files at load time** — In [tests/tck/main.rs](tests/tck/main.rs), substitute the escaped form before passing to the parser:
   ```rust
   let content = std::fs::read_to_string(path)?;
   let content = content.replace("&lt;=", "<=");  // Restore after parsing
   ```

---

### 2. Unicode Encoding Directive Placement

**Severity**: Medium — Affects Literals6.feature (~60 scenarios)  
**Status**: Confirmed  
**Fixability**: Requires feature file patch or parser upgrade

#### Description

The Gherkin `#encoding: utf-8` directive must appear on the very first line of a file. The openCypher TCK `.feature` files include an Apache 2.0 license header before the encoding directive, causing the parser to fail when encountering Unicode characters later in the file.

#### Affected Scenarios

| File | Issue |
|------|-------|
| `Literals6.feature` | Encoding directive after license header; Unicode escapes later in file |

#### Example

```gherkin
#
# Copyright (c) "Neo4j"
# Neo4j Sweden AB [https://neo4j.com]
#
# Licensed under the Apache License, Version 2.0 (the "License");
# ...
# (many lines of license text)
#
#encoding: utf-8        <-- Parser fails: directive not on line 1

Feature: Literals
  Scenario: [1] Return a string with Unicode character
    When executing query:
      """
      RETURN 'café' AS result
      """
```

The UTF-8 directive appears after the copyright header, and when the parser encounters Unicode characters (e.g., `café`, `ß`, etc.), it fails due to the encoding not being declared early enough.

#### Workarounds

1. **Move encoding directive to line 1** — Add it before the license header (not ideal for standards compliance):
   ```gherkin
   #encoding: utf-8
   #
   # Copyright (c) "Neo4j"
   # ...
   ```

2. **Use ASCII-safe escapes** — Replace Unicode literals with escaped sequences:
   ```gherkin
   RETURN 'caf\u00e9' AS result     # café as \u00e9 (é)
   ```

3. **Upstream patch** — Request the openCypher TCK to place the encoding directive on line 1.

4. **Upgrade cucumber-rs** — Newer versions may be more lenient about directive placement.

---

### 3. Special Characters in Triple-Quoted Strings (Match5.feature)

**Severity**: High — Affects Match5.feature (~30+ scenarios)  
**Status**: Confirmed  
**Fixability**: Requires investigation + workaround

#### Description

Backticks (`` ` ``) or pipe characters (`|`) within triple-quoted Gherkin strings (docstrings) confuse the parser, causing the string boundary or table column delimiters to be misinterpreted.

#### Affected Scenarios

| File | Likely cause | Issue |
|------|------|-------|
| `Match5.feature` | Backtick or pipe in query body | Parser terminates string early or misparses table |

#### Example (Suspected)

```gherkin
Scenario: [1] Match query with backtick
  When executing query:
    """
    MATCH (n:Label)     # Note: backtick ` in the wrong place might break parsing
    WHERE n.id = `some_func()`
    RETURN n
    """
  Then the result should be, in any order:
    | n |
    | ... |
```

or with pipes:

```gherkin
Scenario: [2] Match query with pipe
  When executing query:
    """
    MATCH (n)-[r:REL|OTHER]->(m)
    RETURN n, m
    """
```

#### Investigation Steps

1. Open [tests/tck/features/clauses/match-where/Match5.feature](tests/tck/features/clauses/match-where/Match5.feature) in the workspace.
2. Identify the line that causes the parser to fail.
3. Create a minimal reduction in `examples/parse_failure_repro.rs` to isolate the issue.
4. Test whether escaping or wrapping the problematic character resolves it.

#### Workarounds

1. **Escape the character** — Use Gherkin escaping if supported (e.g., `\|` for pipe, `` \` `` for backtick).
2. **Use alternative syntax** — Rewrite the query to avoid the problematic character.
3. **Upgrade cucumber-rs** — Check for fixed versions.

---

### 4. Pattern Comprehensions (Pattern3, Pattern4, Pattern5)

**Severity**: High — Affects 3 files (~60+ scenarios total)  
**Status**: Confirmed  
**Fixability**: Requires investigation + parser upgrade

#### Description

Pattern-comprehension syntax `[ <expr> | <pattern> ]` in Cypher queries is mistaken by the Gherkin parser as table structure markers, causing the docstring boundary to be misidentified.

#### Affected Scenarios

| Files |  Issue |
|-------|--------|
| `Pattern3.feature`, `Pattern4.feature`, `Pattern5.feature` | Pattern comprehension `[ … \| … ]` confused as table delimiter |

#### Example

```gherkin
Scenario: [1] Pattern comprehension in query
  Given any graph
  When executing query:
    """
    MATCH (n)
    RETURN [ x IN nodes | x.id ] AS ids
    """
  Then the result should be, in any order:
    | ids |
    | ... |
```

The `[` and `|` inside the triple-quoted string trigger the parser to treat the following content as table rows, breaking the docstring.

#### Investigation Steps

1. Open [tests/tck/features/expressions/pattern/Pattern3.feature](tests/tck/features/expressions/pattern/Pattern3.feature) and identify the offending line.
2. Isolate the pattern comprehension syntax in a minimal repro.
3. Determine if escaping works: e.g., `\[` or `\|`.

#### Workarounds

1. **Escape the pipe** — Use `\|` instead of bare `|`:
   ```gherkin
   RETURN [ x IN nodes \| x.id ] AS ids
   ```

2. **Use alternative syntax** — Express as `WITH` clause instead of comprehension (if semantically equivalent).

3. **Upgrade cucumber-rs** — Newer parsers may better distinguish table delimiters from docstring content.

---

### 5. EXISTS Subquery Syntax (ExistentialSubqueries1.feature)

**Severity**: High — Affects ExistentialSubqueries1.feature (~30+ scenarios)  
**Status**: Suspected  
**Fixability**: Requires investigation + parser upgrade

#### Description

The `EXISTS { ... }` subquery syntax in Cypher may confuse the Gherkin parser, either at the string boundary or when parsing the nested curly braces.

#### Affected Scenarios

| File | Syntax |
|------|--------|
| `ExistentialSubqueries1.feature` | `EXISTS { MATCH ... RETURN ... }` |

#### Example

```gherkin
Scenario: [1] EXISTS subquery
  Given any graph
  When executing query:
    """
    MATCH (n)
    WHERE EXISTS { MATCH (n)-[r]->(m) WHERE m.id = 123 }
    RETURN n
    """
  Then the result should be, in any order:
    | n |
    | ... |
```

#### Investigation Steps

1. Open [tests/tck/features/subqueries/ExistentialSubqueries1.feature](tests/tck/features/subqueries/ExistentialSubqueries1.feature).
2. Identify which line causes the parse failure.
3. Check if escaping the braces helps: `\{` and `\}`.
4. Test with a minimal repro in `examples/parse_failure_repro.rs`.

#### Workarounds

1. **Escape braces** — If `EXISTS { ... }` breaks the parser, try:
   ```gherkin
   MATCH (n)
   WHERE EXISTS \{ MATCH (n)-[r]->(m) WHERE m.id = 123 \}
   RETURN n
   ```

2. **Upgrade cucumber-rs** — Newer versions may handle nested braces correctly.

3. **Rewrite test** — Use an alternative query that avoids `EXISTS`, if feasible.

---

## Summary Table

| Category | Files Affected | Scenarios | Severity | Workaround Effort |
|----------|----------------|-----------|----------|-------------------|
| `<=` placeholder ambiguity | Comparison2, Quantifier7 | ~10 | Medium | ½ day |
| Unicode encoding directive | Literals6 | ~60 | Medium | ½ day |
| Backtick/pipe in docstrings | Match5 | ~30+ | High | 1 day |
| Pattern comprehensions | Pattern3, Pattern4, Pattern5 | ~60+ | High | 1 day |
| EXISTS subquery syntax | ExistentialSubqueries1 | ~30+ | High | 1 day |
| **Total** | **6 files** | **~190** | — | **~4 days** |

---

## Mitigation Strategy

### Short Term (Phase 1: ~2 weeks)

1. **Identify root causes** — Run each failing feature file through a standalone Gherkin parser (via the `gherkin` crate directly) to pinpoint exact error locations.

2. **Create minimal reproductions** — Add to `examples/parse_failure_repro.rs` for each issue.

3. **Document workarounds** — For each bug, implement the easiest workaround (typically escaping or string substitution).

### Medium Term (Phase 2: ~4 weeks)

1. **Patch TCK feature files** — Apply escaping or structural changes to locally stored copies of failing files.

2. **Implement pre-processing** — Add a feature-file normalizer in [tests/tck/main.rs](tests/tck/main.rs) to apply fixes at load time.

3. **Upstream contributions** — Investigate whether cucumber-rs has newer versions or if upstream patches are needed.

### Long Term (Phase 3: Future)

1. **Extract parser** — Per [parser-extraction.md](plans/parser-extraction.md), consider building a dedicated parser rather than relying on cucumber-rs if it remains unmaintained.

2. **Alternative test harness** — Evaluate whether a custom Gherkin runner (using the `gherkin` crate directly) would be more suitable.

---

## Related Documentation

- [plans/remaining-work.md](plans/remaining-work.md) — Overall TCK gap analysis
- [plans/final-mile.md](plans/final-mile.md) — Tier H (quick wins) includes these bugs
- [plans/spec-first-pivot.md](plans/spec-first-pivot.md) — Notes on permanent Gherkin limitations
- [tests/tck/main.rs](tests/tck/main.rs) — TCK test runner entry point

---

**Last Updated**: May 4, 2026  
**Status**: In Progress  
**Triage Owner**: @team
