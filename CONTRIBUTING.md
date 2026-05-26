# Contributing to Nettablering/arnis-fork

Thank you for considering a contribution. This is the Worldbuilders fork of
[louis-e/arnis](https://github.com/louis-e/arnis), extended with a Roblox
tile emitter. Contributions to the generation pipeline are welcome and
rewarded — see the contributor ladder below.

## Developer Certificate of Origin (DCO) — no CLA required

We use **DCO sign-off** instead of a contributor licence agreement. This is
the same mechanism used by the Linux kernel. Before submitting a pull request,
please read the [Developer Certificate of Origin](https://developercertificate.org/)
(v1.1) in full. By adding a `Signed-off-by` line to each commit you certify
that you have the right to submit the contribution under the project's licence.

Add the sign-off to every commit:

```
git commit -s -m "feat(roblox): improve archetype detection for terrace houses"
```

This produces:

```
feat(roblox): improve archetype detection for terrace houses

Signed-off-by: Your Name <your.email@example.com>
```

Pull requests without a `Signed-off-by` line on **every** commit will not be
merged. The DCO GitHub App enforces this automatically.

Corporate contributors whose legal teams require a bespoke agreement may reach
out to `legal@klokk.studio` to negotiate one.

## Commit style

We follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).

```
<type>(<scope>): <short imperative summary>

[optional body]

[optional footers — Signed-off-by: REQUIRED]
```

**Types:** `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `chore`, `ci`

**Scopes:** `roblox`, `minecraft`, `luanti`, `core`, `cli`, `osm`, `ci`, `deps`

Examples:

```
feat(roblox): add biome-aware water tile colouring
fix(core): clamp elevation deltas to prevent terrain spikes
perf(osm): cache OverpassQL responses in /tmp to skip redundant fetches
docs(roblox): document RobloxEmitter struct fields
```

Breaking changes: add `!` after the type/scope and a `BREAKING CHANGE:` footer.

## Branch model

| Branch | Purpose |
|---|---|
| `main` | always releasable; protected; direct push blocked |
| `feat/<slug>` | new features; branch from `main`, PR back to `main` |
| `fix/<slug>` | bug fixes; same flow |
| `chore/<slug>` | maintenance (deps, CI, docs); same flow |
| `upstream-sync` | automated weekly rebase from `louis-e/arnis main` |

Keep branches short-lived. Squash-merge is the default for `feat/` branches.
Merge commits are used for `upstream-sync` to preserve upstream history.

## Pull request checklist

Before opening a PR:

- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo test --workspace` passes
- [ ] `cargo audit` reports no known vulnerabilities (install with `cargo install cargo-audit`)
- [ ] Every commit has a `Signed-off-by:` line
- [ ] Commit messages follow Conventional Commits
- [ ] PR description explains *why*, not just *what*
- [ ] New public items have rustdoc comments
- [ ] No anti-cheat or game-economy logic touches this repo (see SECURITY.md)

## Contributor ladder

| Tier | Criteria | Rewards |
|---|---|---|
| Drive-by | 1 merged PR | In-game Worldbuilders Contributor badge, name in CREDITS.md |
| Regular | 5+ merged PRs in 6 months | 5 000 Robux stipend (~$17.50 devex) or Stripe cash equivalent, OSS Contributor Discord role, beta access |
| Maintainer | Invited by steering committee; commit access | 25 000 Robux/quarter, name in game splash screen, design-vote weight |
| Core | Employed or contracted | Salary or retainer |

Claim your Drive-by badge by opening an issue linking your merged PR. Robux
stipends are paid quarterly; cash-equivalent via Stripe on request.

## Security issues

Do **not** file security vulnerabilities as public issues. See SECURITY.md for
the private disclosure address and embargo policy.

## Code of conduct

This project is governed by the Contributor Covenant 2.1 — see CODE_OF_CONDUCT.md.

## Governance

The project uses a BDFL model for the first 18 months (Rolf Klokk as final
decision-maker), transitioning to a 3-person steering committee once we have
10+ active maintainers. Details in GOVERNANCE.md (coming in a follow-up PR).
