# Harness evaluation notes

Sources checked September 9, 2026. These notes collect sources, metrics, and comparison conditions referenced by the Acpira README.

## What the evidence supports

A harness runs the agentic loop: it calls the model, executes tools, returns observations, and manages the context and conditions for continuation. Evaluations can therefore measure different outcomes for unchanged model weights. Model capability, harness behavior, task design, and resource budgets jointly affect the result; none of the studies establishes a universal performance ceiling determined by the harness alone.

Acpira uses ACP to keep model interaction and execution in the selected CLI.

## Comparisons referenced in the README

| Source | Design | Evidence and interpretation |
| --- | --- | --- |
| [Artificial Analysis Coding Agent Index](https://artificialanalysis.ai/agents/coding-agents) | Independent evaluation of full coding agents. The current suite combines DeepSWE, Terminal-Bench 2.1, and SWE-Atlas-QnA, with three attempts per task. | The dedicated harness view holds Claude Opus 4.7 fixed. Performance, token use, cost, and runtime are reported separately. The overall leaderboard also varies models and settings, so its entire ranking is not a controlled harness comparison. Component benchmarks have changed over time; current scores should not be combined with older launch tables. |
| [FrontierHarness Eval v1.0](https://frontierharness.org/) · [public repository](https://github.com/frontier-harness-eval/eval) | Runta evaluation: Kimi K3, 30 software-engineering tasks, 9 harnesses, 12 configurations, 360 trials. | Observed pass rates span 50.0%–66.7%. The website and repository use inconsistent labels for some headline cost figures; the README cites pass rates only. Successful-task median cost, all-task cost, and total cost divided by passes are different metrics. The small, fixed task set does not establish a universal ordering. |
| [Composio: eight harnesses](https://composio.dev/content/best-ai-agent-harnesses) | Kimi K3 via OpenRouter, maximum reasoning, common hosted MCP tools, 25 business-application tasks. One valid scored attempt per task and harness. | Pass rates span 68%–88%. Estimated total API costs span $9.28–$35.37 on a shared 24-task subset, using OpenRouter list prices. These are cross-application workflows, not primarily coding tasks. Model-provider, tool, subscription, and repeated-run differences limit extrapolation. |
| [PawBench v1.0](https://github.com/agentscope-ai/PawBench) · [team analysis](https://tongyilab.substack.com/p/the-harness-gap-what-we-learned-from) | AgentScope/OpenJudge: 9 models × 3 harnesses × 150 tasks. Includes text, multimodal, skills, and web workloads. | Qwen3.6-35B-A3B scores 68.3 in QwenPaw, 68.2 in OpenClaw, and 56.7 in Hermes in the repository's overall table. These are composite scores using automated and model-assisted grading. The authors also develop QwenPaw. Text-only scores in the blog use a different task slice and must not replace the overall numbers. |
| [Harness-Bench](https://arxiv.org/html/2605.27922v1) · [code and data](https://github.com/Qihoo360/harness-bench) | Research preprint from Peking University and Qiyuan Tech: 106 offline tasks × 8 backends × 6 configurable harnesses, plus a separate Codex reference, totaling 5,194 trajectories. | Across the shared model pool, aggregate scores span 52.4–76.2. The composite includes completion, security, and process signals. Native tools, prompts, and recovery behavior vary with each configuration. This is a configuration-level comparison averaged over models, not a 23.8-point single-model pass-rate improvement. Codex is reported separately because its model was fixed. |

## Additional context-management experiment

[Same Model, Different Harness: Different Coding-Agent Results](https://arxiv.org/html/2608.26218) is an author-run Yuj harness study published as a preprint. It compares two configurations of the same harness, holding the model and task fixed within each pair. The treatment shortens older tool results under context pressure and responds to stalled execution.

For Qwen3.6-35B-A3B on 169 SWE-bench Verified tasks, with a 20,480-token context window and a 480-second endpoint, complete solutions increased from 43 to 72. Mean per-task fail-to-pass fraction increased from 28% to 49%; this metric is the fraction of required failing tests repaired, not the task pass rate. The package includes several interventions, so its result does not isolate context shortening alone. Under wider context windows, some comparisons show much smaller differences. The study is evidence for configuration sensitivity under specified constraints.

## Kimi K3: tools and harnesses are separate comparisons

The [official Kimi K3 model card](https://github.com/MoonshotAI/Kimi-K3#3-evaluation-results) reports both tool-free and tool-augmented results, as well as scores obtained with different coding harnesses.

| Comparison | Published result | Meaning |
| --- | --- | --- |
| HLE-Full, without / with tools | 43.5 / 56.0 | An official tool-augmentation comparison. It changes available capabilities; it does not isolate Kimi Code or any particular harness. |
| ZeroBench, without / with Python | 23.0 / 41.0, pass@5 | Another tool-augmentation comparison. These are pass@5 scores, not single-attempt coding success rates. |
| DeepSWE v1.1, Kimi Code / mini-SWE-agent | 67.5 / 67.3 | Both runs use harnesses. The small reported gap provides no evidence of a large native-harness advantage on this benchmark. |
| Kimi Code Bench 2.0, Kimi Code / Claude Code | 72.9 / 73.7 | Official in-house benchmark. Claude Code is slightly higher for K3 in this comparison. |

“Bare model” should be defined for each experiment. Tool-free inference, a minimal tool loop, and a general-purpose agent framework are different conditions. In particular, mini-SWE-agent is itself a harness.

## Long-running autonomous research

[Prime Intellect: Measuring Autonomous AI Research](https://www.primeintellect.ai/blog/measuring-autonomous-research) evaluates nanoGPT optimizer research across 18 models and 153 autonomous runs, with multiple seeds and selected promising runs continued for days. Prime Intellect also develops Prime Agent.

The published best validated K3 result closes 52.2% of the gap between the baseline and human record in Prime Agent, versus 45.8% in Kimi Code. **These percentages measure the human-record gap closed, not task pass rates.** The result table includes different run durations, and the study reports substantial variance and changes to launch/recovery behavior. The headline values are descriptive outcomes, not an equal-budget causal estimate of the harness effect. The interactive equal-budget view and run traces provide the relevant context.

## Citation maintenance

Keep source, model, task set, metric, and comparison conditions attached to each number. Recheck current leaderboard definitions before updating snapshots. Vendor reports and harness-author studies should retain their attribution.
