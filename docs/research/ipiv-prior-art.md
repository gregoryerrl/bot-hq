# IPIV Prior-Art Survey (Phase T-0.5)

**Captured:** 2026-05-09
**Purpose:** Inform Phase T-1/T-2 implementation of the IPIV pipeline (Investigate → Plan → Implement → Verify, with cross-model bilateral verification) by surveying production multi-agent orchestration systems.

---

## Executive summary

- **Orchestrator-worker is the dominant production pattern.** Anthropic's research system (Opus lead + Sonnet subagents in parallel) reports 90.2% gain over single-agent Opus on internal evals, attributing 80% of variance to token-budget allocation rather than model choice ([Anthropic engineering](https://www.anthropic.com/engineering/multi-agent-research-system)). Validates IPIV as N specialized agents fanned out from a coordinator.
- **State-machine frameworks (LangGraph) outperform conversational chat-graph (CrewAI, early AutoGen)** for production reliability — LangChain's 2026 State of Agent Engineering report cites that "over 60% of Agent production incidents relate to state management" ([eastondev.com](https://eastondev.com/blog/en/posts/ai/20260424-langgraph-agent-architecture/)). Bot-hq's hub.db + R-rule machinery is closer to LangGraph than to CrewAI.
- **Verification-as-separate-agent is converging.** AutoGen has explicit `Critic`; Anthropic uses a separate Citation Agent; CrewAI tutorials standardize Researcher → Critic → Writer. Bot-hq's "cross-model bilateral" goes further by treating divergence-as-signal first-class — no surveyed system does this.
- **Sandbox + per-agent tool restriction is table-stakes.** Claude Agent SDK exposes `tools`/`disallowedTools` per subagent; AutoGen mandates `DockerCommandLineCodeExecutor`; CrewAI deprecated `allow_code_execution` in favor of E2B/Modal ([CrewAI agents](https://docs.crewai.com/concepts/agents)). Bot-hq should adopt per-agent capability-narrowing as hard requirement.
- **Hook lifecycle is an underused leverage point.** Claude Agent SDK ships ~15 hook events (`PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `PostToolBatch`, `Stop`, `SubagentStart`, `SubagentStop`, `PreCompact`, `WorktreeCreate`, …) ([Agent SDK hooks](https://code.claude.com/docs/en/agent-sdk/hooks)). Bot-hq's R-rules (R33/R36/R40) already mirror this — formalize the isomorphism in T-1.

---

## Per-system findings

### Claude Agent SDK
- **Architecture.** `query()` + `ClaudeAgentOptions` carrying `allowedTools`, `agents`, `hooks`, `mcpServers`, `permissionMode`, `resume`. `AgentDefinition` fields: `description`, `prompt`, `tools`, `disallowedTools`, `model` (`'sonnet'`/`'opus'`/`'haiku'`/`'inherit'`), `skills`, `memory`, `mcpServers`, `maxTurns`, `background`, `effort` (`low`/`medium`/`high`/`xhigh`/`max`), `permissionMode` ([subagents docs](https://code.claude.com/docs/en/agent-sdk/subagents)).
- **State.** Sessions persisted as JSONL; resumable via `resume: sessionId`; subagents resumable via captured `agentId` + same session. Subagent transcripts survive parent compaction.
- **Inter-agent comm.** Strict orchestrator-worker via the `Agent` tool (renamed from `Task` in v2.1.63). Only channel parent→subagent is the Agent tool's prompt string; subagents inherit project `CLAUDE.md` + tool definitions but not parent conversation/system-prompt/tool-results. **Subagents cannot spawn subagents** (single-level enforced).
- **Tool-use.** Built-ins: Read/Write/Edit/Bash/Monitor/Glob/Grep/WebSearch/WebFetch/AskUserQuestion. MCP via `mcpServers`; tool names follow `mcp__<server>__<action>`.
- **Verification.** No first-class critic; userland pattern (define `code-reviewer` subagent with read-only tools). Hooks enforce policy adversarially: `PreToolUse` returns `permissionDecision: "deny"` and most-restrictive-wins across hooks.
- **Production.** Hooks: `PreToolUse`/`PostToolUse`/`PostToolUseFailure`/`PostToolBatch`/`UserPromptSubmit`/`Stop`/`SubagentStart`/`SubagentStop`/`PreCompact`/`PermissionRequest`/`SessionStart`/`SessionEnd`/`Notification`/`Setup`/`TeammateIdle`/`TaskCompleted`/`ConfigChange`/`WorktreeCreate`/`WorktreeRemove`. Async-output mode for fire-and-forget. Bedrock/Vertex/Foundry auth.
- **Applicability.** Highest. `AgentDefinition` is the right shape for IPIV roles; hook lifecycle is the right shape for R-rules; `SubagentStart`/`SubagentStop` map to hub.db span emission.

### CrewAI
- **Architecture.** `Agent` (role + goal + backstory + tools + memory + LLM), `Task` (description + expected_output + context), `Crew` (process + agents + tasks). Agent fields include `allow_delegation`, `respect_context_window`, `max_iter` (default 20), `max_execution_time`, `function_calling_llm` ([agents docs](https://docs.crewai.com/concepts/agents)).
- **State.** `Process.sequential` chains outputs as next-task context. `Process.hierarchical` requires `manager_llm` or `manager_agent`. `Flows` add explicit state persistence ([processes docs](https://docs.crewai.com/concepts/processes)).
- **Inter-agent comm.** Manager-mediated (hierarchical) or output-as-context (sequential). `allow_delegation=True` lets agents call other agents by role-name. **Caveat:** community report ([Towards Data Science 2026](https://towardsdatascience.com/why-crewais-manager-worker-architecture-fails-and-how-to-fix-it/)) indicates hierarchical mode "does not effectively coordinate agents; instead executes sequentially, leading to incorrect reasoning, unnecessary tool calls, and extremely high latency."
- **Tool-use.** Pydantic `BaseTool`. `allow_code_execution` **deprecated** — docs direct users to E2B/Modal sandbox services.
- **Verification.** Userland Researcher + Critic + Writer; no native critic primitive.
- **Applicability.** Moderate. Borrow role/goal vocabulary; avoid the broken hierarchical manager-agent abstraction.

### LangGraph
- **Architecture.** `StateGraph(State)` with TypedDict state; nodes are state→state-update functions; `add_edge`/`add_conditional_edges` connect them. `compile()` produces executable graph ([workflows-agents](https://docs.langchain.com/oss/python/langgraph/workflows-agents)).
- **State.** Centralized state through nodes; `MemorySaver` (dev) / `PostgresSaver` (prod) for checkpoint persistence; threads-for-state + runs-for-execution separation.
- **Inter-agent comm.**
  - **Send API** (`from langgraph.types import Send`): orchestrator-worker fan-out — return `[Send("worker", {"input": x})]` to dispatch parallel workers writing back to shared keys.
  - **Command** primitive: combined state-update + routing in one return value.
  - **Supervisor pattern** via `langgraph-supervisor`: `create_supervisor()` + `create_handoff_tool()` (configurable `handoff_tool_prefix`); message-history modes `"full_history"` / `"last_message"`; `add_handoff_messages` flag; `create_forward_message_tool()` passes worker reply unmodified ([langgraph-supervisor-py](https://github.com/langchain-ai/langgraph-supervisor-py)).
  - Multi-level: compiled supervisor graphs work as worker agents in higher-level supervisors.
- **Verification.** `evaluator-optimizer` is a documented first-class pattern. Human-in-the-loop via `interrupt()` (pause graph, await external input, resume from checkpoint).
- **Production.** Highest among open frameworks. Native checkpointing, time-travel debug, deployable runtime.
- **Applicability.** High. The Send/Command/StateGraph triad maps cleanly onto IPIV: state holds artifact-under-development; nodes are I/P/I/V roles; conditional edges route Verify → Plan/Implement/done. Use this vocabulary internally even without adopting the library.

### AutoGen / AG2
- **Architecture.** v0.4 rewrite (now AG2): event-driven, async-first. Layers Studio/AgentChat/Core/Extensions. `RoutedAgent` base; canonical pair `Assistant(RoutedAgent)` + `Executor(RoutedAgent)`. Runtime `SingleThreadedAgentRuntime()` or `GrpcWorkerAgentRuntime` (distributed). Registration via `Agent.register(runtime, agent_id, factory)` ([code-execution doc](https://microsoft.github.io/autogen/stable//user-guide/core-user-guide/design-patterns/code-execution-groupchat.html)).
- **State.** Event-driven via `publish_message(Message, DefaultTopicId())`, `@message_handler` + `@default_subscription` decorators; per-agent state decoupled from delivery. FSM `GroupChat` allows user-specified speaker transitions ([FSM blog](https://microsoft.github.io/autogen/0.2/blog/2024/02/11/FSM-GroupChat/)).
- **Inter-agent comm.** GroupChat with selector (LLM or rule) deciding next speaker; round-robin and FSM variants. Coder + Executor pair is canonical sandbox pattern.
- **Tool-use.** `DockerCommandLineCodeExecutor(work_dir=...)` (sandboxed) or `LocalCommandLineCodeExecutor` ("not recommended due to the risk of running LLM generated code"). `McpWorkbench` for MCP.
- **Verification.** Explicit `Critic` agent — "double check plans, claims, and code from other agents and provide feedback, while also checking whether plans include verifiable information such as source URLs" ([Critic example](https://microsoft.github.io/autogen/0.2/docs/notebooks/agentchat_groupchat_vis/)). Composition: User → Planner → Engineer ↔ Executor with Critic interjecting.
- **Applicability.** Moderate-high. Event-driven runtime + gRPC mode is closer to bot-hq's daemon than LangGraph's in-process model. Adopt Coder/Executor + mandatory Docker for Implement; adopt Critic prompt pattern (verifiable claims + source URLs) for Verify.

### OpenAI Swarm
- **Architecture.** Stateless; `Agent` (`instructions`, `functions`, `model`) + `Result` (with optional `agent` for handoff and `context_variables` for state-update). Replaced by OpenAI Agents SDK for production ([Swarm GitHub](https://github.com/openai/swarm)).
- **Inter-agent comm.** Handoff-by-return-value: agent function returns an `Agent` → execution transfers. Only the last handoff function executes if multiple are called.
- **Verification.** None native — control-flow library only.
- **Applicability.** Low as framework, high as *handoff-pattern reference*. The "function returns next-agent" idiom is the cleanest expression of explicit handoff and the right shape for IPIV transitions (Investigator returns Planner, Planner returns Implementer, Implementer returns Verifier, Verifier returns Implementer or done). Easier to reason about than AutoGen's selector-based GroupChat.

### Anthropic multi-agent research system (production case study)
- **Architecture.** Lead Opus + 3-5 parallel Sonnet subagents per query; subagents execute 3+ tools concurrently. Lead decomposes into objective + output format + tool guidance + boundary per subagent. Filesystem-based handoff (subagents write to disk, parent reads pointers, not contents).
- **State.** Lead saves research plan to persistent memory before context fills (200K limit); long-horizon conversations use intelligent compression and fresh-subagent handoffs.
- **Verification.** Separate **Citation Agent** identifies exact source locations for claims (post-hoc). LLM-as-judge rubric: factual accuracy, citation accuracy, completeness, source quality (primary vs secondary), tool efficiency.
- **Cost.** Agents use ~4× tokens of chat; multi-agent ~15× chat. Three factors explain 95% of variance: token usage (80%), tool-call count (secondary), model choice (tertiary). Production tracing without conversation-content monitoring (privacy). Rainbow deployments avoid breaking running agents.
- **Failure modes catalogued.** Endless subagent spawning → embed scaling rules; duplicate work → detailed task descriptions with clear divisions; overly specific search queries → start broad, narrow progressively; tool misselection → explicit heuristics; context overflow → external memory; state mutation errors → deterministic retry + checkpoints; non-determinism in debugging → full production tracing without conversation monitoring.
- **Applicability.** Direct. Adopt lead-Opus / worker-Sonnet split for IPIV (Verifier = Opus for adversarial weight; Investigator/Implementer = Sonnet for cost). Adopt filesystem-based subagent output (already aligned with hub.db + artifact-on-disk). Adopt the failure-mode catalog as R-rules. Budget ~15× single-agent token cost as Phase T-1 cost-model floor.

---

## Cross-cutting patterns

### DB read-only agentic patterns
- **Right primitive: Postgres `pg_read_all_data` predefined role** (PostgreSQL 14+). Grants SELECT on all tables/views/sequences + USAGE on all schemas; does *not* bypass RLS ([PostgreSQL docs](https://www.postgresql.org/docs/current/predefined-roles.html), [Cybertec writeup](https://www.cybertec-postgresql.com/en/finally-a-system-level-read-all-data-role-for-postgresql/)).
- **Caveat.** Database-cluster-scoped (reads all DBs); for single-DB restriction use per-DB grants. LangChain's `create_sql_agent` historically had no built-in restriction ([issue #11243](https://github.com/langchain-ai/langchain/issues/11243)); workarounds use readonly connection users + SELECT-only `create_sql_query_chain`.
- **Recommended bot-hq pattern.**
  1. Connect as a dedicated `bot-hq-readonly` user that is a member of `pg_read_all_data` and nothing else.
  2. Force `SET TRANSACTION READ ONLY` at session start (defense in depth).
  3. Wrap connection in a tool that pre-flights via SQL parser (sqlglot/pgsanity) — reject non-SELECT and multi-statement queries.
  4. Per-subagent connection isolation (no shared transaction state).

### Sandbox patterns for code-execution + browser-repro
- **Container-per-execution (AutoGen / managed agents):** `DockerCommandLineCodeExecutor(work_dir=...)`, disposable container per Bash invocation, network-disabled by default. Agents read error output and iterate ([Playwright Test Agents](https://playwright.dev/docs/test-agents)).
- **Testcontainers + Playwright with tracing-on:** `testcontainers-node-playwright` bundles a Playwright-ready container; Playwright MCP logs every decision (DOM observation, LLM in/out, action), replayable step-by-step ([Bug0 writeup](https://bug0.com/blog/playwright-test-agents)). MCP sends *structured commands* — AI never executes arbitrary code.
- **Recommended bot-hq pattern.** For IPIV Verify on browser-repro: Playwright via Testcontainers with tracing always on, trace-replay artifact stored in hub.db (or filesystem with hub.db pointer). For non-browser code execution: container-per-Bash with network-disabled and explicit allowlisted mounts.

### Multi-model orchestration (cross-model bilateral)
- **Established: model routing by cost/capability tier.** Frontier models for planning/verification; cheaper models for execution/bulk. Routing conditional on output confidence or task-flagged complexity ([aithority.com 2026](https://aithority.com/machine-learning/from-gpt-5-5-to-deepseek-v4-how-developers-are-building-smarter-ai-agents-with-multi-model-routing-in-2026/), [MindStudio](https://www.mindstudio.ai/blog/ai-model-orchestration-smart-model-cheaper-sub-agents)).
- **Established: grader-as-separate-instance (LLM-as-judge).** Separate Claude instance evaluates output against criteria; if it fails, grader identifies what needs to change and agent iterates.
- **Not established:** treating cross-model **divergence as first-class signal** rather than as noise to resolve via one-side-wins. Surveyed systems either use one model end-to-end (no divergence), use a grader to pick a winner (divergence collapsed to verdict), or use ensembling/voting (divergence averaged out). **Bot-hq's "cross-model bilateral" — Verifier on a different model family than Implementer, with divergence preserved as audit-evidence — appears to be a novel design.** Closest prior art is Anthropic's Citation Agent (same model family, structurally different verification — partial divergence signal) and RL-on-orchestration-traces research ([arXiv 2605.02801](https://arxiv.org/html/2605.02801)) which treats agent interactions as reward-design substrate but doesn't isolate cross-family divergence.

---

## Recommendations for bot-hq Phase T-1 / T-2

### Adopt
1. **Claude Agent SDK's `AgentDefinition` schema** as the canonical per-agent config shape for R51 model-config-load (fields: `description`/`prompt`/`tools`/`disallowedTools`/`model`/`maxTurns`/`effort`/`permissionMode`/`mcpServers`).
2. **Hook lifecycle isomorphism** between bot-hq R-rules and Agent SDK hook events — explicitly map R33 → PreToolUse, R36 → Stop, R40 → Notification, etc.
3. **Orchestrator-worker for IPIV** with single-level hierarchy (subagents cannot spawn subagents, per Agent SDK rule). Coordinator = IPIV controller; workers = Investigator/Planner/Implementer/Verifier.
4. **Filesystem-based artifact handoff** (Anthropic research-system pattern) — subagents write to artifact files; parent receives pointer + summary, not contents.
5. **AutoGen Coder/Executor + mandatory `DockerCommandLineCodeExecutor`** for Implement when code-execution is involved. Local execution disallowed by R-rule.
6. **Postgres `pg_read_all_data` + SET TRANSACTION READ ONLY + SQL-parser pre-flight** for any IPIV agent querying customer databases.
7. **LangGraph StateGraph mental model** for documenting IPIV control-flow even if not adopting the library — state-as-explicit-object + node-as-function + conditional-edges is the clearest vocabulary.
8. **Anthropic's failure-mode catalog as R-rules** — most are not yet codified: scaling-rule-in-prompts (anti-spawn-loop), task-boundary-in-handoff (anti-duplicate), broad-then-narrow search, checkpoint-on-state-mutation, conversation-content-free tracing.
9. **Per-subagent token/turn budgets** (`maxTurns`, `effort`) as hard caps with hub.db span emission on hit. Token-budget allocation explains 80% of performance variance per Anthropic — make it explicit and observable.

### Avoid
1. **CrewAI's hierarchical-process pattern** — community-documented to fall through to sequential with high latency.
2. **Multi-level subagent spawning** — explicitly disallowed by Claude Agent SDK; known recipe for endless-spawn failure.
3. **`LocalCommandLineCodeExecutor` or equivalent** — Microsoft, Anthropic, CrewAI all converged on "do not let agents execute code directly on host." Bot-hq should not regress.
4. **Conversational chat-graph as primary control-flow** (early AutoGen GroupChat, CrewAI sequential chat) — for IPIV the state-graph is structured enough that explicit edges beat selector-LLM-picks-next-speaker.
5. **Synchronous all-the-way** (Anthropic's documented limitation: "lead agent can't steer subagents mid-task"). If T-2 can afford async-with-cancel, that's the production-mature direction.
6. **Untyped state passing between IPIV roles** — adopt typed artifact schemas (Pydantic/Go struct) so divergence between Implementer output and Verifier expectation is type-error not runtime surprise.

### Extend with bot-hq-specific innovations
1. **Cross-model bilateral verification with divergence-as-signal.** No surveyed system does this. Implementer on family A (Claude Sonnet); Verifier on family B (DeepSeek or GPT-class); divergence *not* resolved by winner-take-all judge but logged as structurally meaningful event; arbitration step (third model OR human) invoked when divergence exceeds threshold. Hub.db schema includes `divergence_event` row type with both outputs verbatim. Produces a corpus of divergence events that becomes training/evaluation data for prompt + model-pairing tuning over time.
2. **R-rule discipline machinery as production-grade hook system.** Agent SDK's hooks are per-process-instance; bot-hq's hub-routed hub.db-persisted R-rule events should be the multi-agent equivalent — hooks fire, hub.db records, dashboards observe, discipline-machinery is queryable across agents and across time. Make this the explicit Phase T-1 differentiator vs. "just use the SDK directly."
3. **IPIV phase-doc + ratchet-ledger as first-class state.** No surveyed system has the equivalent of bot-hq's phase-doc (canonical-scope-per-R10 SCOPE-LOCK) and ratchet-ledger (forward-only progress). T-2 should ensure these survive the rebuild as first-class hub.db tables, not filesystem-only artifacts.
4. **Per-IPIV-step verification rubric in hub.db** (not just final-output evaluation). Anthropic uses LLM-as-judge dimensions (factual accuracy, citation accuracy, completeness, source quality, tool efficiency) — make these queryable per IPIV step so degradation is detectable per-role rather than only end-to-end.

---

## References

- [Claude Agent SDK overview](https://code.claude.com/docs/en/agent-sdk)
- [Claude Agent SDK subagents](https://code.claude.com/docs/en/agent-sdk/subagents)
- [Claude Agent SDK hooks](https://code.claude.com/docs/en/agent-sdk/hooks)
- [Anthropic: How we built our multi-agent research system](https://www.anthropic.com/engineering/multi-agent-research-system)
- [CrewAI agents](https://docs.crewai.com/concepts/agents)
- [CrewAI processes](https://docs.crewai.com/concepts/processes)
- [Why CrewAI's Manager-Worker Architecture Fails (TDS)](https://towardsdatascience.com/why-crewais-manager-worker-architecture-fails-and-how-to-fix-it/)
- [LangGraph workflows-and-agents](https://docs.langchain.com/oss/python/langgraph/workflows-agents)
- [langgraph-supervisor-py](https://github.com/langchain-ai/langgraph-supervisor-py)
- [LangGraph State Management in Practice (eastondev 2026)](https://eastondev.com/blog/en/posts/ai/20260424-langgraph-agent-architecture/)
- [AutoGen code-execution group chat](https://microsoft.github.io/autogen/stable//user-guide/core-user-guide/design-patterns/code-execution-groupchat.html)
- [AutoGen Critic example](https://microsoft.github.io/autogen/0.2/docs/notebooks/agentchat_groupchat_vis/)
- [AutoGen FSM GroupChat blog](https://microsoft.github.io/autogen/0.2/blog/2024/02/11/FSM-GroupChat/)
- [OpenAI Swarm GitHub](https://github.com/openai/swarm)
- [PostgreSQL predefined roles](https://www.postgresql.org/docs/current/predefined-roles.html)
- [Cybertec: pg_read_all_data writeup](https://www.cybertec-postgresql.com/en/finally-a-system-level-read-all-data-role-for-postgresql/)
- [LangChain SQL agent security issue #11243](https://github.com/langchain-ai/langchain/issues/11243)
- [Playwright Test Agents docs](https://playwright.dev/docs/test-agents)
- [Bug0: Playwright Test Agents writeup](https://bug0.com/blog/playwright-test-agents)
- [Testcontainers Playwright module](https://testcontainers.com/modules/playwright/)
- [aithority.com: multi-model routing 2026](https://aithority.com/machine-learning/from-gpt-5-5-to-deepseek-v4-how-developers-are-building-smarter-ai-agents-with-multi-model-routing-in-2026/)
- [MindStudio: AI model orchestration](https://www.mindstudio.ai/blog/ai-model-orchestration-smart-model-cheaper-sub-agents)
- [RL for LLM Multi-Agent Systems via Orchestration Traces (arXiv 2605.02801)](https://arxiv.org/html/2605.02801)
