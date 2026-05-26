# Alpha feedback collection

Three feedback surfaces, ordered by signal-density (highest first):

1. **In-game surveys** — three time-triggered surveys per tester
   (see `in-game-survey-design.md`). Highest fidelity because they fire at the
   moment of the experience.
2. **Discord** — surface-specific channels (see `discord-server-layout.md`)
   plus the bi-weekly 30-min Stage-1 voice call.
3. **GitHub Issues, private repo** (`klokk-nettablering/alpha-feedback-private`)
   — long-form bug reports & feature wishes. Templates in
   `github-issue-templates/`. Repo is private; testers added by email after NDA
   signature. Embargoed until launch + 30 days.

Stage-2 testers also have an in-game `/feedback` slash command that writes a
DataStore row Rolf reviews daily.

Stage-3 testers feed into PostHog surveys triggered after a 30-min session;
results land on the public-facing pulse dashboard.

The churn monitor (../churn-monitor/) consumes the Discord + Roblox-analytics
side-channel to detect testers who have gone quiet for ≥5 days and auto-DMs.
