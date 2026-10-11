# claude-plus

Token, cache and cost visibility for Claude Code.
claude-plus draws a live pane beside the conversation with what each session costs, what fills its context, when its cache goes cold, how fast it uses your subscription windows, and tips that save tokens.
A local daemon reads your Claude Code transcripts and keeps their history; nothing leaves your machine.

![The live view of the claude-plus pane docked beside a Claude Code conversation](docs/assets/live.png)

## Install

claude-plus runs on macOS and Linux and needs Claude Code 2.1.296 or newer.

Install the binary with Homebrew:

```bash
brew install gdevxy/tap/claude-plus
```

Or with the install script, which puts it in `~/.local/bin`:

```bash
curl -fsSL https://github.com/gdevxy/claude-plus/releases/latest/download/install.sh | sh
```

The script checks the archive against the release's `SHA256SUMS`, which guards against a broken download but comes from the same place as the archive, so it does not prove who built it.
It runs nothing until it has arrived whole, so a dropped connection installs nothing.

Then add the pane to Claude Code:

```bash
claude-plus install
```

`claude-plus install` shows the `claude plugin` commands it runs and asks before it runs them, because Claude Code records the marketplace and the plugin in `~/.claude/settings.json`; `--yes` agrees without asking.
It then starts the daemon, which imports your history in the background.
Start a new Claude Code session: the pane docks beside the conversation when the terminal is at least 144 columns wide, and `/plus` opens it at any width.

The mod alone can also be installed from Claude Code's prompt; it still needs the `claude-plus` binary on `PATH`:

```
/plugin install plus --marketplace gdevxy/claude-plus
```

## The pane

The pane has four views, and the keys 1 to 4 switch between them.

### 1. Live

The current session at a glance: what it costs, how long the cache stays warm, and how fast you use your 5h and 7d subscription windows.
Next prompt compares what your next message would cost on each model, with the cache warm and after it expires, so you know what a model switch really costs.

![Live view: cost, cache countdown, next prompt price per model and plan limits](docs/assets/live.png)

### 2. Context

What fills the context window, by category, and how far you are from auto-compact.
It lists the largest items and the skills, tools and memory files you configured but have not used in 14 days, with what each costs every turn.

![Context view: context by category, largest items and unused skills](docs/assets/context.png)

### 3. History

Cost per day and model, cache hit ratio, what the cache saved you, and your top projects and sessions.
`r` cycles the range between 24h, 7d, 30d and 90d, and `g` switches between cost and tokens.

![History view: cost by day, cache hit ratio and top projects](docs/assets/history.png)

### 4. Insights

Ranked tips drawn from your own sessions, each with the weekly saving it promises and how confident claude-plus is.
Each tip names the sessions and files behind it, so you can see where the tokens went.

![Insights view: ranked tips with the weekly saving each one promises](docs/assets/insights.png)

### Alerts

claude-plus shows a toast when the cache is about to go cold, when the 5h limit would run out before its reset at the current pace, and when the context nears auto-compact.
Turn them off with the `alerts` option of the mod.
The `chime` option, off by default, also plays a soft chime with the cache alert.
Both options are set in Claude Code's plugin settings for `plus`.

## Upgrade

Upgrade the binary with `brew upgrade claude-plus` or by running the install script again.
The next Claude Code session restarts the daemon on the new version and updates the mod in place; that session still shows the pane it loaded, and the sessions started after it show the new one.
An older `claude-plus` found first on `PATH` never replaces a newer mod: `claude-plus install` keeps a newer mod in `~/.claude-plus/marketplace` and says so.
When the `plus` mod comes from another marketplace, such as the GitHub one above, `claude-plus install` asks before it points the `claude-plus` marketplace at its own folder.
If that mod is newer than the binary, it stops instead and changes nothing: upgrade `claude-plus`, or run `claude plugin uninstall plus@claude-plus` first to switch to the binary's version.

## Uninstall

```bash
claude-plus uninstall
```

This removes the mod and its marketplace from Claude Code and stops the daemon; `--purge` also deletes `~/.claude-plus` with the history claude-plus keeps.
claude-plus marks its data directory with a `.claude-plus-data` file only when it created the directory or found it empty.
It never stores its data in a `data.db` that another program made: it creates `data.db` with a claude-plus stamp in its header, and refuses any `data.db` without that stamp, leaving the file as it is and asking you to move it aside.
`--purge` deletes only a marked directory, and in it only `data.db` when the file carries the stamp, and the daemon's files when they were created after the mark.
It keeps anything else it finds there, and names it.
It refuses a symlink (with or without a trailing `/`), your home directory, a directory that holds or sits inside the Claude Code directory, and one without the mark.
Run `claude-plus uninstall` before you remove the binary with `brew uninstall claude-plus`, or delete `~/.local/bin/claude-plus`.

## Commands

| Command | What it does |
|---|---|
| `claude-plus install` | Add the pane to Claude Code and start the daemon. |
| `claude-plus uninstall [--purge]` | Remove the pane and stop the daemon, and with `--purge` delete the data. |
| `claude-plus stats [--days N]` | Print daily cost per model. |
| `claude-plus export --format json\|csv` | Print stored usage per day, project, account and model. |
| `claude-plus doctor` | Check the binary, daemon, mod, Claude Code version and transcript retention. |
| `claude-plus import` | Read new transcripts into the store, with progress. |
| `claude-plus reindex` | Rebuild derived tables from stored records. |
| `claude-plus daemon [--ensure [--unless-stopped]]` | Run the daemon, or start it detached unless one answers; with `--unless-stopped`, leave one stopped by `daemon stop` stopped. |
| `claude-plus daemon stop` | Stop the running daemon and keep it stopped: open panes leave it alone until `/plus`, a new session or `--ensure` starts it. |

## Privacy

claude-plus makes no network requests.
The daemon listens only on a Unix socket in `~/.claude-plus`, readable by you alone, and never edits Claude Code's files.
`CLAUDE_PLUS_HOME` moves the data directory and `CLAUDE_CONFIG_DIR` the Claude Code directory.
`CLAUDE_PLUS_HOME` must be an absolute path; a relative `CLAUDE_CONFIG_DIR` is read from the working directory, as Claude Code reads it.

## License

MIT
