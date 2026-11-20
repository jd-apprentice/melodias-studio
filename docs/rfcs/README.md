# RFCs

Design documents for work that is bigger than one change. An RFC describes the intended behavior; the code and the user docs describe what exists today. Status values: `proposed`, `accepted`, `partly implemented`, `implemented`, `rejected`.

| RFC | Status | Summary |
| --- | --- | --- |
| [0001](0001-adversary-review.md) | partly implemented | Candidate pools, hard gates, the cloud judge and video assembly. Pools, judge and assembly exist; gates and the retry loop do not. |
| [0002](0002-deterministic-scoring.md) | partly implemented | Technical and editorial score, `melodias score`, the SQLite history, the read-only dashboard and the deterministic retry loop exist; the five editorial axes and their weights do not. |

The split between deterministic and editorial scoring in 0002 depends on the judge from 0001 and replaces its single opaque score.
