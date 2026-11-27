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

## 2026-11-27 21:34 UTC
- **Session:** 54cfa081-c549-42dd-8e5f-d065fb40c0cb (/Users/user/Developer/Developer/respond/native-view-engine)
- **Observation:** User corrected holding Slice 2B-3 remediation/landing waiting for Neon Kali review and 2A-1+2B-0 landing: 'No always land everything immediately as it is available don’t wait, you can still do review passes they just work in parallel in background'.
- **Proposal:** In AGENTS.md (Delegation & landing), make explicit: never hold landing a buildable commit or applying available fixes waiting for an in-flight review pass or sibling branch; land immediately onto main as each buildable milestone finishes and let reviews/follow-up fixes run asynchronously in parallel.

## 2026-11-27 21:37 UTC
- **Session:** 54cfa081-c549-42dd-8e5f-d065fb40c0cb (/Users/user/Developer/Developer/respond/native-view-engine)
- **Observation:** User noted: 'Btw in general something doesn’t have to follow all objections about a review but instead take a step back and reevaluate the whole approach in general'. Across Slice 2B-1/2B-2/2B-3 reviews, dozens of per-host review objections pointed to 5 shared root causes in libnve_runtime/nve-abi.
- **Proposal:** In AGENTS.md (iterative dual review), add explicit guidance: never blindly checklist-patch every review objection; step back, synthesize cross-cutting root causes, question whether the whole approach should be simplified or lifted to a shared layer, and reject or supersede weak/symptomatic review suggestions.
