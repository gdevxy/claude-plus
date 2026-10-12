# Morning check for the limit, cache lifetime and subagent model tips

Run this by hand in a real terminal, after Tasks 1 to 4 of the plan are on `main`.

## Setup

Build the daemon and restart it, so it moves the store to schema 19 and reads every subagent's meta file.

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo build --release --locked
./target/release/claude-plus daemon stop
./target/release/claude-plus daemon --ensure
./target/release/claude-plus doctor
PATH="$PWD/target/release:$PATH" claude --plugin-dir "$PWD/mod"
```

## Checklist

- [ ] `sqlite3 ~/.claude-plus/data.db "select agent_type, count(*) from agents group by 1 order by 2 desc"` lists your agent types (for example `general-purpose`, `Explore`), and no row holds a description.
- [ ] Press `4`: when your subagents ran on Opus, a tip reads `Save ~$x/week: run <type> subagents on Sonnet 5.5`, its evidence lists up to three agent types with `runs on Opus 5.5 cost ...`, and `Subagents drew n% of your spend`.
- [ ] The `?` of that tip says how it was priced and how to set `model:`, at `low confidence` for a type that writes code.
- [ ] No cache lifetime tip shows while your main thread is on the 1-hour cache and often waits on subagents; when one shows, its evidence names `Main thread` or `Subagents` and its `?` names `promptCacheTtl` or `subagentPromptCacheTtl`.
- [ ] After a day where a 5-hour or weekly limit ran out, the first tip is `Run subagents on ...`, `Move routine work to ...` or a pacing tip, without a saving, above every priced tip, and its first evidence line says how often the limit ran out and how long before it reset.
- [ ] A session of another account never shows that account's limit tip.
- [ ] At 40, 60 and 100 columns, in `dark`, `light` and `dark-ansi`, no new tip is cut and no value splits from its unit across rows.
