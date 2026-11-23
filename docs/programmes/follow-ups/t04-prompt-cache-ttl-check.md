+++
type = "task"
mode = "afk"
status = "open"
blocked_by = []
repos = ["greenbelay"]
due = "2026-11-30"
+++
# Did the 1-hour prompt-cache TTL cut the re-cache cost after idle gaps

## Question

`promptCacheTtl: "1h"` was set in `settings.json` on 2026-11-23. Did it cut
what sessions pay to write their cache again after an idle gap?

## Count

Rerun `~/.local/state/sess-audit/tokscan/gap.py` over the week since. The
30-day baseline: $13.4k of re-cache after gaps over 5 minutes, $1.6k after
gaps over 55 minutes. Expected: the 5-to-55-minute share mostly gone.

Filed from inbox line 8efa358a (2026-11-23).
