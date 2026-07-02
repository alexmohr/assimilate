# File Change Patterns

!!! note "New in Assimilate"

    File change pattern configuration was added in release X.Y.Z.

File change patterns let you control how warnings about files that change during a backup are handled. By default, any file that is modified while being backed up generates a warning. You can configure specific path patterns to:

- **ignore** — suppress the warning entirely
- **warn** — keep the warning (default)
- **fatal** — fail the backup

## Configuration

File change patterns can be configured at the **schedule level** (applies to all targeted agents) or **per agent** (overrides the schedule-level patterns for specific hosts).

![File change patterns section](assets/screenshots/file-change-patterns.png)

### Schedule-level patterns

Navigate to **Schedules** → select a schedule → **File Change Patterns** section.

Each line contains an optional action keyword followed by a glob pattern:

```text
/tmp/logs ignore
/etc/config fatal
/var/www/cache warn
```

If no action is specified, `warn` is assumed:

```text
/tmp/logs          ← equivalent to `/tmp/logs warn`
```

### Per-agent patterns

When a schedule targets multiple agents, you can toggle **Configure per agent** and provide different patterns for each host.

## Action Reference

| Action | Behavior |
|--------|----------|
| `ignore` | The warning is silently discarded; the backup continues with no alert |
| `warn` | The warning is preserved in the report (default; backward compatible) |
| `fatal` | The backup is stopped and reported as failed with an error message |

## Pattern Syntax

Patterns use the same shell glob matching as the rest of Assimilate:

- `*` matches any number of characters within a path component (does not match `/`)
- `?` matches any single character

The pattern is matched against the full warning message text. For example, a warning message like `/var/log/nginx/access.log: file changed while we backed it up` can be matched with a pattern like `access.log: file changed while we backed it up` or the simpler `*access.log*`.

## Backward Compatibility

Existing schedules without file change patterns continue to work as before — all file changes produce warnings. The feature is entirely opt-in.
