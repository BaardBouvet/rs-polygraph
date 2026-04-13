# TCK Final Four: 459/463 → 463/463

**Prerequisite**: All phases from `tck-100-plan.md` complete (459/463, 99.1%)

**Goal**: Close the remaining 4 scenarios to reach 100% TCK compliance.

**Core change**: Introduce `TranspileOutput` multi-phase enum + collect→UNWIND
peephole optimization.

---

## Scenarios

| # | Scenario | Query | Blocker |
|---|----------|-------|---------|
| 1 | Unwind1 [5] | `MATCH (row) WITH collect(row) AS rows UNWIND rows AS node RETURN node.id` | Runtime UNWIND of `collect()` |
| 2 | Unwind1 [12] | `MATCH (a:S)-[:X]->(b1) WITH a, collect(b1) AS bees UNWIND bees AS b2 MATCH (a)-[:Y]->(b2) RETURN a, b2` | Runtime UNWIND of `collect()` + re-MATCH |
| 3 | Return2 [14] | `MATCH ()-[r]->() DELETE r RETURN type(r)` | DELETE + RETURN in one statement |
| 4 | Match8 [2] | `MATCH (a) MERGE (b) WITH * OPTIONAL MATCH (a)--(b) RETURN count(*)` | MERGE clause |

---

## Part A — Collect-then-UNWIND Peephole (scenarios 1, 2)

### Insight

Both scenarios follow the pattern:

```cypher
WITH ... collect(X) AS list
UNWIND list AS item
```

The net effect is an identity: `item` binds to the same values as `X`. The
`collect()` groups values into a list, then `UNWIND` immediately iterates
that list back into individual rows. This round-trip can be eliminated at
compile time — no multi-phase execution needed.

### Scenario 1 walkthrough

```cypher
MATCH (row)
WITH collect(row) AS rows
UNWIND rows AS node
RETURN node.id
```

After peephole elimination of collect→UNWIND:

```cypher
MATCH (row)
RETURN row.id        -- 'node' is just 'row' renamed
```

SPARQL:

```sparql
SELECT ?node_id WHERE {
  ?node <base:__node> <base:__node> .
  OPTIONAL { ?node <base:id> ?node_id }
}
```

### Scenario 2 walkthrough

```cypher
MATCH (a:S)-[:X]->(b1)
WITH a, collect(b1) AS bees
UNWIND bees AS b2
MATCH (a)-[:Y]->(b2)
RETURN a, b2
```

After peephole elimination — `b2` is just `b1` renamed, grouped per `a`:

```cypher
MATCH (a:S)-[:X]->(b2)
MATCH (a)-[:Y]->(b2)
RETURN a, b2
```

SPARQL (simple join):

```sparql
SELECT ?a ?b2 WHERE {
  ?a a <base:S> .
  ?a <base:X> ?b2 .
  ?a <base:Y> ?b2 .
}
```

### Implementation

In `translate_query()`, after parsing WITH + UNWIND pairs, detect:

```
WITH ... collect(VAR_X) AS VAR_LIST ...
UNWIND VAR_LIST AS VAR_ITEM
```

When found:
1. Drop both the `collect()` aggregation and the `UNWIND` clause.
2. Create a variable alias: `VAR_ITEM → VAR_X`.
3. Carry forward all other WITH items unchanged.
4. Apply the alias to subsequent clauses (MATCH, WHERE, RETURN).

**Edge cases**:
- Non-adjacent collect/UNWIND (computation between them) → don't optimize.
- Multiple items in WITH alongside `collect()` → only eliminate the
  collect/UNWIND pair; keep other WITH items as GROUP BY keys.
- `collect(DISTINCT x)` → same optimization applies (DISTINCT is preserved
  on the renamed variable).

**Location**: `src/translator/cypher.rs`, new function:

```rust
fn try_eliminate_collect_unwind(
    clauses: &mut Vec<Clause>,
) -> HashMap<String, String>  // returns alias map: UNWIND var → original var
```

Called at the start of `translate_query()` before clause iteration.

**Estimated size**: ~40 lines.

---

## Part B — Multi-Phase TranspileOutput (scenarios 3, 4)

### API change

```rust
// src/target/mod.rs

/// The result of transpiling a Cypher/GQL query.
pub enum TranspileOutput {
    /// A single SPARQL query string (SELECT, CONSTRUCT, ASK).
    Single(String),
    /// An ordered sequence of SPARQL operations that must be executed
    /// sequentially. SELECT results from earlier phases can be injected
    /// into later phases as VALUES bindings.
    MultiPhase(Vec<QueryPhase>),
}

pub struct QueryPhase {
    pub sparql: String,
    pub kind: PhaseKind,
}

pub enum PhaseKind {
    /// Execute as SELECT; capture result bindings.
    Select,
    /// Execute as SPARQL UPDATE (INSERT/DELETE).
    Update,
}
```

### Backwards compatibility

`Transpiler::cypher_to_sparql()` continues to return `Result<String, _>` for
single-phase queries. Add a new method:

```rust
impl Transpiler {
    pub fn cypher_to_sparql_plan(
        cypher: &str,
        engine: &dyn TargetEngine,
    ) -> Result<TranspileOutput, PolygraphError> { ... }
}
```

The existing `cypher_to_sparql()` calls the new method and unwraps `Single`,
returning an error for `MultiPhase` queries when the caller doesn't support it.

### Scenario 3: DELETE + RETURN

```cypher
MATCH ()-[r]->() DELETE r RETURN type(r)
```

Transpile to two phases:

**Phase 1 (Select)** — capture bindings before deletion:
```sparql
SELECT (REPLACE(STR(?_r_pred), "^.*[/#]", "") AS ?type_r)
WHERE {
  ?_r_src ?_r_pred ?_r_dst .
  FILTER(?_r_pred != <base:__node>)
}
```

**Phase 2 (Update)** — delete the matched triples:
```sparql
DELETE WHERE {
  ?_r_src ?_r_pred ?_r_dst .
  FILTER(?_r_pred != <base:__node>)
}
```

Return Phase 1 results to the caller.

**Implementation**: In `translate_query()`, when both `DELETE` and `RETURN`
clauses are present, emit a `MultiPhase` output. The SELECT phase is built
from the MATCH + RETURN. The UPDATE phase is built from the MATCH + DELETE.

### Scenario 4: MERGE

```cypher
MATCH (a) MERGE (b) WITH * OPTIONAL MATCH (a)--(b) RETURN count(*)
```

Transpile to two phases:

**Phase 1 (Update)** — conditional insert for MERGE:
```sparql
INSERT DATA { _:b <base:__node> <base:__node> }
```
But only if no node `b` exists. MERGE with unconstrained `(b)` and existing
nodes in the store is a no-op. The translator detects this:

- MERGE `(b)` with no labels/properties → unconditionally matches any node
  → no insert needed → Phase 1 can be skipped entirely.
- MERGE `(b:Label)` → conditional:
  ```sparql
  INSERT { [] a <base:Label> ; <base:__node> <base:__node> }
  WHERE { FILTER NOT EXISTS { ?b a <base:Label> } }
  ```

**Phase 2 (Select)** — the query after MERGE:
```sparql
SELECT (COUNT(*) AS ?count) WHERE {
  ?a <base:__node> <base:__node> .
  ?b <base:__node> <base:__node> .
  OPTIONAL {
    { ?a ?_p ?b . FILTER(?_p != <base:__node>) }
    UNION
    { ?b ?_p ?a . FILTER(?_p != <base:__node>) }
  }
}
```

Return Phase 2 results.

---

## Part C — TCK Runner Changes

Update `executing_query()` in `tests/tck/main.rs` to handle multi-phase:

```rust
#[when(regex = r"^executing query:$")]
async fn executing_query(world: &mut TckWorld, step: &Step) {
    if world.skip { return; }
    let cypher = step.docstring.as_deref().unwrap_or("").trim();

    let output = match Transpiler::cypher_to_sparql_plan(cypher, &ENGINE) {
        Err(e) => { world.query_error = Some(e.to_string()); return; }
        Ok(o) => o,
    };

    let store = world.store.get_or_insert_with(|| OxStore(Store::new().unwrap()));

    match output {
        TranspileOutput::Single(sparql) => {
            execute_select(store, &sparql, world);
        }
        TranspileOutput::MultiPhase(phases) => {
            for phase in &phases {
                match phase.kind {
                    PhaseKind::Select => execute_select(store, &phase.sparql, world),
                    PhaseKind::Update => {
                        if let Err(e) = store.0.update(&phase.sparql) {
                            world.query_error = Some(e.to_string());
                            return;
                        }
                    }
                }
            }
        }
    }
}
```

Extract the existing query-execution logic into `execute_select()` (~5 lines
of refactoring).

---

## Side Effects Assertions

Scenarios 3 and 4 assert side effects (`-relationships 1`, `no side effects`).
The TCK runner currently ignores all side-effect assertions. No change needed
for passing, but for completeness, the runner could compare store triple counts
before and after execution. This is optional and out of scope for this plan.

---

## Dependency Graph

```
Part A (peephole)     ── independent, no API change
Part B (multi-phase)  ── adds TranspileOutput enum + new API method
Part C (TCK runner)   ── depends on Part B
```

Parts A and B can be implemented in parallel.

---

## Estimated Effort

| Part | Lines | Effort |
|------|-------|--------|
| A — Collect-UNWIND peephole | ~40 | < 1 session |
| B — TranspileOutput + DELETE/MERGE translation | ~100 | 1 session |
| C — TCK runner multi-phase | ~20 | < 1 session |
| **Total** | **~160** | **1-2 sessions** |

---

## Expected Result

| Before | After |
|--------|-------|
| 459/463 (99.1%) | **463/463 (100%)** |
