---
name: unbiased-feature-brainstorm
description: Zero-priming web-search feature brainstorm via a fresh research agent (no repo reads), then diff against the codebase. Use when discovering new tasks or maturity gaps.
---

1. Delegate a fresh T3 `research` agent (`isolation: "shared"`, never `gateway/gpt-6-luna`):
   - **Forbid reading/searching the local repo** (zero bias).
   - **Zero priming**: name no features, frameworks, or this project—only the general category (e.g. *mature cross-platform UI framework*).
   - **Require web search**; output concrete, edge-case-aware **Tier 1 (table stakes) / Tier 2 (production) / Tier 3 (delighters)** directly in the reply.
2. After it returns, diff its list against the codebase (**Implemented / Partial / Missing**) and propose top gaps to the user.
