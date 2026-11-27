# Improvement backlog

Noted by agents with note_improvement. Dreaming moves each note into its report when it reads it.

## 2026-11-27 07:07 UTC
- **Session:** 26f760be-2e50-4d7d-98e4-7ea3960429de (/Users/user/Developer/Developer/respond/native-view-engine)
- **Observation:** User instructed: "for future use pi opus 5.5 for implementation, but keep the double neon/opus review/plan approach". AGENTS.md currently defaults implementation delegates to gemini-4-neon.
- **Proposal:** Update AGENTS.md Delegation & landing model policy so implementation defaults to anthropic/claude-opus-5-5 (pi) while keeping dual gemini-4-neon (antigravity) + anthropic/claude-opus-5-5 (pi) for research, planning, and adversarial reviews.

## 2026-11-27 20:17 UTC
- **Session:** 54cfa081-c549-42dd-8e5f-d065fb40c0cb (/Users/user/Developer/Developer/respond/native-view-engine)
- **Observation:** User explicitly corrected three orchestration anti-patterns: (1) "Remove integration branch feature and in future land everything directly immediately on main once commited", (2) "Don't get lost on review cycles continue work in parallel and let agents work with almost finished things, the details are gonna get sorted out in passes, but don't block progress", and (3) "Why tf are you so eagerly trying to prevent merge conflicts. They are totally natural and an result of good architecture overhaul because an agent saw need to change other files. NEVER EVER MAKE THE SAME MISTAKE OF PRE DEFINING THE ALLOWED PATHS AND ALLOW DEVIATING FROM THE PLAN". Coordinator had repeatedly boxed delegates into "Strictly Owned Paths" / "Do NOT edit files outside these paths" and gated landing behind multi-pass review cycles on a long-lived integration branch.
- **Proposal:** Update AGENTS.md Delegation & Landing rules to explicitly state: (1) Never use a long-lived integration branch — land every committed, buildable change directly onto main and push immediately, landing review refinements as follow-up commits on main instead of stalling progress behind review cycles; (2) NEVER pre-define "allowed/owned paths" or forbid delegates from editing shared files to avoid merge conflicts — merge conflicts are a normal result of holistic architectural fixes and are reconciled on main; always empower every delegate to deviate from the plan and fix any shared foundation directly.
