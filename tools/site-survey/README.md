<!-- agent-updated: 2026-10-13T22:00:00Z -->

# Site survey

How the candidate list in `tools/harvest/sites.csv` was built (decisions.md U31). Data lives outside the
repo in `$REX_SURVEY_DIR` (default `~/projects/scratch/rex-site-survey`): `raw/` downloads, `work/`
intermediate JSONL, `out/` results.

| Step | Script | Output |
|---|---|---|
| 1. Rankings | download Tranco (`top-1m.csv`) and Majestic Million (`majestic.csv`) into `raw/`; `chinaz.py` scrapes the Chinaz overall and industry lists | `raw/` |
| 2. Pool | `pool.py`: merge by registrable domain, mark infrastructure domains | `work/pool.jsonl` |
| 3. Probe | `probe.py`: fetch each homepage once (title, language, schema.org types, RSS, bot walls, a sample article link) | `work/probe.jsonl` |
| 4. Classify | `classify.py`: LLM labels (category, rex schema, value, access) via `$REX_LLM_ENDPOINT` | `work/labels.jsonl` |
| 5. Select | `select.py`: keep sites whose pages map to a rex record; suggest tiers | `out/candidates.csv` (copied to `tools/harvest/sites.csv`), `out/sites.json` |

Each script's docstring has its `uv run --with ...` line. All steps resume where they stopped.
