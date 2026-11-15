# Migrating to the new data names

greenpiste 5.0.0 renames its data names: the `GREENPISTE_*` variables, the
`greenpiste.*` git config keys, the `greenpiste` state directories, the
`greenpiste-*-v1` schemas and the `clear-relay@greenpiste` plugin.

The first greenpiste command after upgrading (a hook, or any subcommand except
`migrate` and `doctor`) migrates the machine once, under a lock. A hook never
waits for the lock and is never blocked or refused by the migration.

## What moves

- The two state roots, `~/.claude/amont-agent` and `<state home>/amont-agent`,
  are each renamed to `greenpiste`, with a symlink left at the old path.
- The `amont.agent.*` git config keys are renamed in the files that hold them.
  Includes are followed whether or not their condition holds. Each key is
  copied, verified and only then removed.
- The clear-relay plugin registration in `settings.json` and its marketplace
  file.

## Markers

Three files in the Claude config directory record the state:

| file | meaning |
|---|---|
| `greenpiste-migrated` | the migration finished; hooks stop checking |
| `greenpiste-migrate-reversed` | `--reverse` ran; a hook never migrates forward while it exists |
| `greenpiste-migrate-failed` | a step failed; it holds the step and what to fix |

## The command

```sh
greenpiste migrate [--reverse] [--dry-run]
```

An explicit run waits up to 30 s for the lock. `--dry-run` changes nothing.
`--reverse` puts everything back, rewriting 5.0 journal and ledger rows to the
old names, and leaves nothing at the new paths.

## Until the migration is done

- Until the done-marker exists, a hook still reads an old `amont.agent.*` key
  when its new twin is absent. The new key wins when both exist. After the
  marker exists the old keys are ignored.
- Old environment variables are no longer read. `greenpiste doctor` lists each
  one that is set, with the name to set instead.
- Old journal rows, review rows and committed `.cases` files are still read
  until the old names are retired.
- The installers accept `AMONT_AGENT_BIN_DIR` and `AMONT_AGENT_VERSION` with a
  warning; the new names are `GREENPISTE_BIN_DIR` and `GREENPISTE_VERSION`.
