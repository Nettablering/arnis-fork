# Alpha Discord server layout (Q243)

Private invite-only Discord. Rolf is admin; three community mods (paid 200 Robux/mo
each) join at Stage 2. Linked from the invite-redemption email.

## Roles

- `@founder` (Rolf)
- `@mod` (Stage-2+)
- `@alpha` — Stage-1 testers (50)
- `@beta` — Stage-2 testers (500)
- `@soft-launch` — Stage-3 testers (5 000)
- `@alumni` — graduated testers; read-only on `#founder-lounge`

## Channels — Stage 1 (private)

```
WELCOME
  #welcome              (rules, NDA reminder, watermark notice)
  #announcements        (founder-only post)
  #stage-1-roll-call    (introduce yourself + device + country)

SURFACES (one per gameplay surface, lets bug tags route)
  #claim-loop
  #build-mode
  #raid-loop
  #ui-readability       (mobile/desktop font + contrast bugs)
  #performance          (FPS, server stalls)
  #onboarding           (first 5 minutes)

REPORTING
  #bugs                 (use !bug <surface> <repro>)
  #feature-wishes
  #anonymous-feedback   (webhook -> Tally form, NOT a chat channel)

GENERAL
  #general
  #off-topic
  #voice-bi-weekly      (voice chan for the 30-min call)
```

## Channels — Stage 2 additions (still NDA-soft)

```
#state-of-beta          (Rolf weekly Loom)
#creator-corner         (creators who plan to clip when embargo lifts)
#founder-lounge         (Stage-1 alumni only; keeps them from drowning Stage-2 chat)
```

## Channels — Stage 3 (public)

```
#announcements
#general
#bugs                   (template-gated by automod)
#clips                  (creators welcome)
#regional-{ph, nz, no}
```

## Backup plan

Discord could false-positive-ban us. Mirror a minimal Guilded server with the same
role + channel structure, plus an email list (`alpha@worldbuilders.app`) we can
fall back to within an hour.
