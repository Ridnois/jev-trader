# Paper trader (`src/trade/`)

- `trade run` runs every `strategies/*.json`, 1-second ticks into `data-v3/`, Jev catalyst recorder (needs the Jev key via `--env-file ../trading/.env.local`).
- `trade monitor` prints health and paper results; paper results never tune a running strategy.
- Promoting writes a strategy file: `lab promote --family <toml> --candidate [--spec ... --as <name>]`. User's decision only.
- Old promoted-file formats must still load.
