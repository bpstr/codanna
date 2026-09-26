# Symbol ranking ablation results

Evidence branch for T05/T06. No runtime ranking change is made here.

## Candidate-loss baseline

The seven-query Assign-style corpus classified three current R0 misses as low-rank
lexical candidates rather than absent corpus:

| Query | Current Hit@5 | Candidate rank in top 200 |
| --- | ---: | ---: |
| calendar settings → `useAccountPresentation` | miss | 51 |
| integration binding → `createIntegrationBinding` | miss | 53 |
| workspace preferences → `resolveWorkspacePreferences` | miss | 50 |

Overall R0 Hit@5 was **4/7**. Test-local distinct query-term coverage produced
4/7 with 4x and 8x candidates, then **7/7** with 16x and with a fixed 200 pool.

## Broader corpus

The 21-query TypeScript/Rust/Go evaluation observed:

- R0: **11/21 Hit@5**, MRR@5 **0.524**.
- R1 with an 80-candidate pool: **20/21 Hit@5**, MRR@5 **0.952**.
- Seven-query holdout: R0 5/7, MRR@5 0.714; R1 **7/7**, MRR@5 **1.000**.
- The remaining `cache-invalidation` owner was still present at lexical candidate
  rank **98**, establishing candidate-budget loss rather than a coverage-rule miss.

This selects a small-query candidate floor above 98 while retaining the existing
hard ceiling of 200. PR #52 uses 128.

## Cost measurement

The fixture also executes repeated local searches at limits 5 and 128 and records
returned candidate counts, serialized-result bytes, and elapsed microseconds.
Timing is observational only: CI latency is not an acceptance gate and serialized
bytes are not an RSS estimate. The purpose is to retain the cost evidence beside
the relevance result rather than claiming that more candidates are free.

The latest exact-source run after adding that measurement is pending at this
checkpoint. Earlier relevance/holdout runs and repository gates are preserved in
the PR history rather than rewritten.
