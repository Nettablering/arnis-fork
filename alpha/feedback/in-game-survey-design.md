# In-game survey design (Q243 + Q322)

Three time-triggered micro-surveys per tester per cohort. Each fires once; result
written to Roblox DataStore key `wb:survey:{user_id}:{minute}` and shipped to the
bake-server `/v1/alpha/survey` endpoint (added in a follow-up; for now Datastore
is canonical and Rolf eyeballs it).

## Trigger minutes

| Minute | Name              | Goal                                          |
|--------|-------------------|-----------------------------------------------|
| 5      | First impression  | Did the player understand what to do?         |
| 30     | First-session end | Why did they stop? Where did the loop drag?   |
| 180    | Retention check   | What made them come back? Or, why didn't they? |

180-minute trigger fires once cumulative playtime crosses 3 hours, not on a single
session — most alpha testers won't sit for 3 hours straight.

## Question format

Each survey is **at most 3 questions**. Two scaled (1–5 stars), one free-text.
Skippable per question. Closed automatically after 30 seconds of inactivity.

### Minute-5

1. *(scale 1–5)* "How clear was it what to do in the first minute?"
2. *(scale 1–5)* "How does the game look on your device right now?"
3. *(free-text, optional)* "Anything visually broken or confusing?"

### Minute-30

1. *(scale 1–5)* "How much fun was that session?"
2. *(scale 1–5, inverted)* "How often did the game feel slow or stuck?"
3. *(free-text, optional)* "What was the most boring 30 seconds of that session?"

### Minute-180

1. *(scale 1–5)* "Did you come back because you wanted to, or because we asked?"
2. *(multiple choice)* "Which surface do you most want us to fix next?" — Claim /
   Build / Raid / UI / Performance / Other.
3. *(free-text, optional)* "If you stopped playing earlier this week, why?"

## Data model

```json
{
  "user_id": 123456789,
  "stage": 1,
  "survey": "minute-30",
  "answered_at": "2026-06-01T10:15:00Z",
  "answers": { "q1": 4, "q2": 2, "q3": "raids load slowly on cellular" },
  "client": { "device": "iphone-se-2", "fps_p50": 28 }
}
```

## Aggregation

Rolf runs `scripts/dump-surveys.py` (TODO sibling of `churn-monitor/`) every
Monday; output appended to `docs/HEARTBEAT.md` under "Alpha pulse".

## Stop conditions

- If average minute-5 score < 2.5 across 10+ responses: pause Stage-1 recruitment
  and ship a fix wave before adding more testers.
- If minute-180 q1 leans "because you asked" >60%: the loop isn't sticky;
  graduation gate fails.
