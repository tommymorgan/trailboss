# Amp orbs and coordination

Research date: **2026-10-14**. Scope: publicly accessible, first-party Amp documentation, announcements, and generated API reference. This is a documentation investigation, not a hands-on product evaluation. No account was created, credentials supplied, compute provisioned, CLI executed, or workload run. Product behavior below means **documented behavior**, not behavior independently exercised.

## Executive findings

1. **An orb is normally an Amp-created, per-thread cloud machine—not a generic name for any remote agent host.** The current setup guide explicitly says it runs in Amp’s cloud. Amp also documents **runners**, which run on machines you supply, and publicly references **self-hosted orbs**, whose full documentation was sign-in-gated during this investigation. Do not collapse those three deployment categories. [Overview](https://ampcode.com/docs/orbs), [setup](https://ampcode.com/docs/orbs/getting-started), [runners](https://ampcode.com/docs/cli/runners), [self-hosted overview](https://ampcode.com/docs/orbs/self-hosted).
2. **Amp is already an orchestration and development environment, not just a model attached to a VM.** Its documented workflow includes persistent thread URLs across web/CLI/phone, environment provisioning, pause/wake, shared terminals, previews and annotations, diffs/files, local sync, agent-to-agent delegation and file transfer, schedules, and webhook-triggered work. [Threads](https://ampcode.com/docs/threads), [overview](https://ampcode.com/docs/orbs), [agent-to-agent](https://ampcode.com/docs/orbs/agent-to-agent), [automations](https://ampcode.com/docs/orbs/automations), [events](https://ampcode.com/docs/orbs/event-driven).
3. **Parallel subagents and communicating remote threads are different mechanisms.** Specialist subagents have separate context but cannot communicate with each other or receive human mid-task guidance; persistent agent threads can exchange messages/files and run in other projects or on runners. Neither messaging nor a new orb automatically transports uncommitted code. [Models and subagents](https://ampcode.com/docs/models-and-subagents), [agent-to-agent](https://ampcode.com/docs/orbs/agent-to-agent).
4. **Both the central-devbox and no-devbox workflows are addressed.** A runner can expose an existing machine to Amp clients; a hosted orb supplies compute when the user has no suitable machine. Shared runners and multiplayer add real multi-human participation, but sharing a runner grants execution as its OS user, not a tenant sandbox. [Runners](https://ampcode.com/docs/cli/runners), [multiplayer](https://ampcode.com/docs/collaborate/multiplayer), [pricing](https://ampcode.com/pricing).
5. **The important caveats are lifecycle, authority, and cost—not just launching agents.** Merely opening an orb thread can resume billing; multiplayer can expose secrets and terminals to the workspace; sibling-thread edits need explicit integration. Claims of unlimited spawning in an August essay must be qualified by current spawn pacing and size restrictions. [Costs](https://ampcode.com/docs/orbs/sizes-and-costs), [multiplayer](https://ampcode.com/docs/collaborate/multiplayer), [agent-to-agent](https://ampcode.com/docs/orbs/agent-to-agent), [August essay](https://ampcode.com/notes/orbs-explained), [current overview](https://ampcode.com/docs/orbs).

## 1. Terminology and deployment boundaries

| Term | What the current sources establish | What it does not establish |
| --- | --- | --- |
| **Thread** | The unit of work: conversation, tool calls, changed files, and a URL. A thread can execute in an orb, on a runner, or in a local CLI session. [Threads](https://ampcode.com/docs/threads) | A thread URL does not mean compute runs on the device displaying it. |
| **Orb** | A fresh isolated environment for each orb thread, with code, plugins and tools; normally created in Amp’s cloud. The security reference calls it a sandboxed cloud machine; the explanatory essay calls orbs VMs/sandboxes and also uses “remote agents” as product shorthand. [Setup](https://ampcode.com/docs/orbs/getting-started), [security](https://ampcode.com/security#orb-data), [essay](https://ampcode.com/notes/orbs-explained) | “Orb” alone does not specify a hypervisor isolation guarantee, dedicated physical host, or full self-hosting of Amp. |
| **Runner** | A running Amp instance on a machine the user already has. Threads use that machine’s directories, tools and logins. Amp does not provision, size, pause or wake the machine. [Runners](https://ampcode.com/docs/cli/runners#runners-vs-orbs) | Selecting a served directory is not filesystem or command confinement; concurrent threads share the machine. |
| **Self-hosted orb** | The official indexed overview says Amp can spawn orbs in a company’s own AWS, Google Cloud or Azure cloud. Current portal docs explicitly reference self-hosted orbs. [Overview, search-index text only](https://ampcode.com/docs/orbs/self-hosted), [portal restrictions](https://ampcode.com/docs/orbs/portals#access-and-lifecycle) | Deployment recipe, commercial eligibility, exact isolation, pricing and control-plane architecture were not accessible. The HTML redirected to sign-in and its Markdown endpoint returned HTTP 401. |
| **Project** | Repository mapping/hosting plus shared settings: repo, secrets, orb size, setup and shipping behavior; can belong to a person or workspace. [Projects](https://ampcode.com/docs/projects) | A project is not itself an execution sandbox or a distinct human-permission tier for every action. |

**Infrastructure detail, not a permanent placement guarantee:** the security reference lists E2B as the provider of ephemeral orb compute; the customization cookbook says the default E2B orbs currently run in GCP `us-west1`. The same reference distinguishes the Amp Server’s thread storage from the orb filesystem. Do not generalize that location to every self-hosted deployment or future orb. [Security](https://ampcode.com/security#infrastructure-service-providers), [customization cookbook](https://ampcode.com/docs/orbs/customizing#mount-external-storage).

## 2. What the human actually does

This is a synthesis of the documented setup and continuation flow, not an invented demo or a claim that these steps were executed.

### Create and launch

1. **Sign in and choose code.** The web interface needs no local install. Connect GitHub for private GitHub repositories and pushing as the user; public Git repositories can be supplied by HTTPS URL. Private non-GitHub hosts need clone credentials. Create a project with an existing repository, start an Amp-hosted blank repository, or choose **No Project** for a clean orb without code. Projects can include additional repositories. [Getting started](https://ampcode.com/docs/orbs/getting-started), [projects](https://ampcode.com/docs/projects).
2. **Choose executor, project and mode in New Thread, then submit.** With **New Orb**, Amp creates the machine, clones code and starts the agent; subsequent threads may reuse a project snapshot. The mode and orb size are selected for the new thread rather than resized/switched mid-thread. [Getting started](https://ampcode.com/docs/orbs/getting-started#3-send-a-prompt), [sizes](https://ampcode.com/docs/orbs/sizes-and-costs), [Dial](https://ampcode.com/docs/the-dial#choose-the-mode-before-the-first-message).
3. **Prepare future environments if needed.** The setup UI can start a thread that writes, exercises and commits executable `.agents/setup` and `.agents/resume` scripts. Review those changes and ship them. This is optional, but avoids repeating dependency setup in every fresh orb. Long-running services belong in `.amp/services.yaml`, not background processes left by setup. [Getting started](https://ampcode.com/docs/orbs/getting-started#4-prepare-the-repository-for-orbs-optional), [customization](https://ampcode.com/docs/orbs/customizing), [portals](https://ampcode.com/docs/orbs/portals).

**CLI/TUI launch is the same remote-work concept:**

```sh
amp -ox "Investigate why the latest CI run on main failed"
amp -ox "Run the performance test matrix" --project my-org/my-repo --orb-size a1.large
```

`-ox` means `--orb-execute -x`: it normally prints a thread URL and exits immediately while work continues on Amp’s servers. The CLI matches the local checkout’s Git remotes to a project; absent a match or explicit project it warns and can start without a repository. This is not an implicit upload of the local dirty working tree. Piped input and `--attach` can supply explicit material. `amp --executor orb` opens an interactive TUI that provisions after the first prompt; the command palette also has **thread: new in orb**. [Spawning orbs](https://ampcode.com/docs/cli/spawning-orbs), [overview](https://ampcode.com/docs/orbs).

### Monitor and intervene

The same thread URL opens across web, CLI and native apps. The sidebar/project pages organize work; Activity can filter visible threads by status, time, author and project. In the thread, **Changes** shows diffs, **Files** opens the environment’s files, **Terminal** gives a shell, and **Portals** opens exposed orb services. [Threads](https://ampcode.com/docs/threads).

The orb terminal shares its filesystem and working copy with the agent and uses a shared tmux session; Amp can inspect its output and embed a live terminal in a reply. The human can inspect processes, run commands and edit alongside the agent. Files can be attached to a message, uploaded through Files, or pasted/dropped into Terminal; direct Files/Terminal uploads are limited to 512 MB per file. Those are different from thread-attachment limits. [Overview](https://ampcode.com/docs/orbs).

When the agent is working, an ordinary new message is **steered after its current step**. `Ctrl+Enter`/`Cmd+Enter` instead queues until the turn finishes; pressing `Esc` twice interrupts. This is stronger intervention than merely reading a transcript. [Threads](https://ampcode.com/docs/threads#inside-a-thread).

Ask the agent to start a development server and provide a portal. Portal annotations send the selected text/page location and available source location back into the thread. Portals require their own proxy authentication, separate from the application’s login. A configured URL is not proof of service readiness; the docs explicitly distinguish a printed link from successful health checks. [Portals](https://ampcode.com/docs/orbs/portals).

### Leave, resume and collect results

A closed laptop does not stop an orb task. Return to the same thread on another computer or phone; native apps notify when the thread is ready for the next message. The current macOS/iOS apps are **beta**, require macOS/iOS/iPadOS 26+, and iOS is publicly available through TestFlight rather than an asserted stable App Store release. No Android-native app was established by the sources read. [Overview](https://ampcode.com/docs/orbs), [native apps](https://ampcode.com/docs/macos-and-ios).

To send another prompt without taking over execution:

```sh
amp threads continue T-… -ox "Continue investigating with the new evidence"
```

It runs on the existing thread’s orb and ignores new `--project`, `--mode`, and `--orb-size` options. `--stream-json` instead stays attached until the turn ends; that variant archives new threads on completion unless `--no-archive-after-execute` is set. `--stream-json-input` is not supported with `-ox`. [Spawning orbs](https://ampcode.com/docs/cli/spawning-orbs).

Results remain in the conversation, files, changes and running services. `amp sync <thread>` mirrors orb changes into a local checkout while the agent continues remotely; do not assume documented sync is a general bidirectional filesystem replication or cross-agent merge protocol. Threads also export Markdown, and creators can export full JSON. [Overview](https://ampcode.com/docs/orbs#sync-changes-to-your-computer), [threads](https://ampcode.com/docs/threads#export-a-thread).

**Shipping is an agent instruction, not an atomic merge service.** The **Ship** button sends a prompt. The default asks the agent to commit, fetch/rebase onto the base branch, test, push directly to it, and archive when appropriate; substantive conflicts require checking with the human. **Push to Branch** and **Custom Ship** support other team policies. A pull-request-only workflow must be selected or requested, not assumed. [Shipping](https://ampcode.com/docs/orbs/shipping).

## 3. Environment state and lifecycle

### Provisioning and snapshots

The documented startup sequence is: find a snapshot for the selected size; reuse a matching one, otherwise run pre-clone, clone/update, pre-setup and the primary repository’s `.agents/setup`; save successful prepared state; apply the current thread environment and workload identity; run `.agents/resume`; start the agent. Additional repositories live under `../repos/` and only the primary setup script runs automatically. [Customization](https://ampcode.com/docs/orbs/customizing#the-orb-lifecycle).

Snapshots are reusable for at most 72 hours before refresh and may be shared across project members. Changes to `.agents/setup` alone do **not** invalidate an existing snapshot; delete the cache when immediate application is required. Setup has a 20-minute timeout and Amp continues startup on timeout/failure without publishing a refreshed snapshot. Processes left by setup are stopped, except services owned by systemd as described in the guide. Resume is run at activation and each wake; Amp waits at most 10 seconds before letting the agent continue while the hook finishes. Scripts must be idempotent. [Customization](https://ampcode.com/docs/orbs/customizing).

Orbs run Debian 12 with Amp, Git/SSH/tmux, `gh`, common JavaScript/Python tools and browser/media utilities; software installed manually persists in that orb, but must enter setup to appear in future orbs. Docker is not installed by default and requires installation plus a supervised daemon service. [Customization](https://ampcode.com/docs/orbs/customizing#installing-software).

### Pause, wake and deletion

An orb pauses only once **both** idle conditions hold: no agent work for 5 minutes and no human interaction for 20 minutes. Long commands and tests count as work; a leftover dev server by itself does not keep the orb awake. Other-agent results, schedules, queued prompts and plugin keep-alive leases can count as work. [Sizes and costs](https://ampcode.com/docs/orbs/sizes-and-costs#when-an-orb-pauses).

Opening an unarchived thread in the web/native app, opening an authorized portal, or sending a message can wake the orb and resume billing. **Orb Saver Mode** prevents merely opening the thread from waking it. Authorized portal traffic, including shared visitors or service callers, starts a new 20-minute interaction window. Archiving immediately pauses the orb; snoozing hides/mutes a thread but does not stop work or schedules. [Costs](https://ampcode.com/docs/orbs/sizes-and-costs), [threads](https://ampcode.com/docs/threads#organize-threads).

Pause is not deletion: the security reference states filesystem and memory remain stored, encrypted at rest, until the thread is deleted; files and services are restored on wake. Explicit thread deletion deletes its orb, usually within seconds, while thread-data deletion can take up to 30 days. There is no separately configurable orb-retention period. Snapshot replacement is not immediate data erasure: replaced snapshots cannot be deleted until dependent orbs are deleted. Enterprise thread ownership/retention differs from personal/non-Enterprise accounts. [Security, Orb Data and Thread Data](https://ampcode.com/security#orb-data).

## 4. Coordination: distinguish the mechanisms

### Specialist subagents are scoped harness delegation

The documented specialists include Search, Oracle, Librarian, Task, and Read Thread. Each has its own context window and tools; main-agent and subagent models can differ. Amp can choose them automatically, or the human can request parallel independent subagent work. Crucially, the docs say subagents **cannot communicate with each other**, the human **cannot guide them mid-task**, they receive the main agent’s supplied context rather than the full conversation, and the parent receives a final summary rather than monitoring every step. “Isolation” here should not be mistaken for a documented separate orb or security sandbox for each specialist. [Models and subagents](https://ampcode.com/docs/models-and-subagents).

### Persistent threads can communicate across environments

A new orb thread has its own conversation, context window, working copy and machine. The documented side-quest pattern is to ask the current agent to start another thread, provide context, request a result, then continue its own task. Fan-out can run several independent investigations, browser checks or builds in parallel. Agents can work in other projects or on a named runner and report back to the originating thread. [Agent-to-agent](https://ampcode.com/docs/orbs/agent-to-agent).

**A message does not move files or commits.** Uncommitted files are not shared automatically. Request explicit send/retrieve operations for schema files, screenshots, generated documents or useful files from an earlier prototype. Amp’s own guidance is to keep constantly coordinating tasks or same-file edits in one thread and delegate tasks with a clear independent result. This is not evidence of an automatic distributed shared working tree, conflict-free merge layer, or transactional task scheduler. [Agent-to-agent](https://ampcode.com/docs/orbs/agent-to-agent).

A **handoff** is context continuation into a fresh thread: “Handoff and …” starts another thread with relevant context. Referencing a thread URL or `@T-…` lets an agent read prior work. This is distinct from transporting an executing process and its memory between arbitrary machines. The docs read do not establish live executor migration for a thread. [Threads](https://ampcode.com/docs/threads).

### Public API reference supports real orchestration primitives

The generated `@ampcode/plugin` reference documents `create_thread`, `list_agent_modes`, `get_thread_status`, `send_thread_message`, and `wait_for_threads` as coordination tools; `Task` is scoped subagent execution and does not accept a per-call agent mode. Its typed `Agent.createThread(...)` accepts local, orb or named-runner execution, returns a thread handle for appending messages, awaiting replies, observing state or cancelling, and creates work independent of the caller. `Agent.run(...)` is a one-shot create/send/wait convenience; its timeout can reject while the child continues to the end of its turn. Child visibility and remaining multiplayer window inherit by default unless explicitly overridden. The project option requires an orb and can select accessible project secrets, so the API warns against taking it from untrusted input. These are published interface contracts, not a source-code audit of their implementation. [Plugin API](https://ampcode.com/docs/plugin-api).

Automations save one schedule and prompt per thread, wake it later/repeatedly, preserve history and can be paused/deleted. A failing run pauses the automation. Event-driven webhooks are a separate plugin mechanism restricted to Amp-managed orbs: persist-before-HTTP-200 acceptance, at-least-once and not strictly ordered delivery, 30-second handlers, retry limits, rate limits and owner-thread affinity. Archiving the owning thread disables its endpoint/delivery; loading the same plugin in a new thread does not transfer ownership. Amp does not verify provider signatures for the plugin author. These are meaningful durable orchestration facilities, but not exactly-once workflow execution. [Automations](https://ampcode.com/docs/orbs/automations), [event-driven orbs](https://ampcode.com/docs/orbs/event-driven).

## 5. Bringing your own machines and adding humans

### Runners: close to the central development machine workflow

Start a runner on the machine that should execute commands:

```sh
amp --no-tui --runner-id my-devbox --remote-control-terminal
```

Then select it in the web composer or address it from another CLI:

```sh
amp -x "Run the tests and report results" --executor runner:my-devbox
```

Stable runner IDs identify live runner processes. It can serve explicit directories or discover Git checkouts/worktrees. The directory selection controls **where a thread starts**, not what files or commands it can access. The runner requires the process and host to remain on; Amp does not supply remote power management. Files/Changes work while connected, Terminal requires its explicit flag, and Portals are currently **not supported** on runners. A `--desktop` option supports macOS screen control and a separate Linux virtual desktop, not capture of the existing Linux desktop. [Runners](https://ampcode.com/docs/cli/runners).

For parallel threads, the private runner UI can create separate Git worktrees, including from a phone. Dirty changes stay in the original checkout. This is working-copy separation, not isolation of OS credentials or hardware resources. Worktrees remain on disk after the thread ends unless explicitly removed; archive-and-remove refuses dirty/untracked or unmerged work. [Runner worktrees](https://ampcode.com/docs/cli/runners#create-a-worktree).

With `--amp-env`, centrally configured secrets augment the runner process environment, with personal > project > workspace > process precedence. This is opt-in. A subtle boundary: environment is managed per served directory, so the latest thread started there sets it for **every thread in that directory**, with directory MCP servers/plugins restarted on changes. Shared runners receive workspace/project values, not the runner owner’s personal Amp secrets—even for their own threads—but ordinary OS-user credentials still remain available. [Runner environment](https://ampcode.com/docs/cli/runners#secrets-env-vars).

### Multiplayer and sharing are different permissions

**Thread viewing:** the current Share UI supports private, workspace, Enterprise group, and unlisted-link audiences. Private workspace threads remain visible to admins. Viewing is not equivalent to contributor control. Projects have separate rights: workspace members can start threads, but only the project creator/admin can change workspace-project settings. [Threads](https://ampcode.com/docs/threads#share-a-thread), [projects](https://ampcode.com/docs/projects#permissions).

**Multiplayer:** enable it for a thread in an orb or shared runner; default duration is 7 days, configurable/endable early. Teammates can prompt together and use the terminal/files/portals where supported. Tagging a member can activate multiplayer. The explicit security warning says **every workspace member** can access the environment, secrets, files and terminal while active. Costs go to the thread owner. Voice/video/screen-sharing **Space** is a separate thread-associated call available without enabling multiplayer. [Multiplayer](https://ampcode.com/docs/collaborate/multiplayer).

**Shared runners:** `--share` exposes the machine to workspace members who can run code as the OS user that started it. Other members’ threads are theirs, visible to the workspace, and not automatically multiplayer. Threads may concurrently overwrite the same directory. Creating directories/projects/worktrees on a shared runner is currently unsupported. Restarting without `--share` revokes sharing and ends multiplayer, but does not reverse file changes or stop programs already launched. Admins can forbid runner sharing; already-running threads lose runner access within five minutes under that admin action. [Sharing a runner](https://ampcode.com/docs/cli/runners#share-a-runner-with-your-workspace).

**Document inconsistency to resolve before deployment:** the Multiplayer page says an owner’s own threads qualify when started in the workspace associated with the shared runner, **whatever project** they use. The runner feature table agrees, but a bullet in the runner sharing section says the owner’s own threads must start in a **workspace project** and other threads stay personal. Do not silently choose the broader permission interpretation. The Share UI’s group-contribution option also needs reconciliation with the multiplayer page’s whole-workspace security warning before claiming least-privilege group-only execution. [Multiplayer](https://ampcode.com/docs/collaborate/multiplayer), [runners](https://ampcode.com/docs/cli/runners), [sharing UI](https://ampcode.com/docs/threads#share-a-thread).

**Ownership is not a multi-owner abstraction:** the sources establish a creator/thread owner charged for multiplayer, workspace/project ownership, admin visibility, and Enterprise-owned retained threads. They do not establish equal co-owners, arbitrary cross-organization collaborators with independent roles, or transferable billing ownership of an active thread. [Multiplayer](https://ampcode.com/docs/collaborate/multiplayer), [projects](https://ampcode.com/docs/projects), [security](https://ampcode.com/security#thread-data).

## 6. Isolation, secrets and network boundaries

Fresh orb-per-thread environments reduce working-copy/resource collisions; this is the documented separation. The reviewed pages do not specify a full tenant-isolation threat model, kernel/hypervisor guarantees, outbound-deny defaults, customer-managed disk keys, or security equivalence between managed and self-hosted orbs. Runners explicitly do **not** supply this kind of sandboxing. [Overview](https://ampcode.com/docs/orbs), [security](https://ampcode.com/security), [runners](https://ampcode.com/docs/cli/runners).

Secrets/environment variables can come from workspace, project and personal settings, in that precedence order with personal winning. Workspace values also apply to personal-project orbs in that workspace. Secrets are hidden after saving and their occurrences are masked in orb shell output, with change history available. Redaction should not be treated as an access-control boundary: code in the environment and multiplayer participants can access credentials; the security reference itself calls pattern-based secret detection best-effort. [Secrets](https://ampcode.com/docs/orbs/handling-secrets), [multiplayer](https://ampcode.com/docs/collaborate/multiplayer), [redaction](https://ampcode.com/security#secret-redaction).

OIDC can replace long-lived secrets with a signed, audience-bound identity token (default ten minutes, configurable 60 seconds–one hour). Thread/user/project/workspace scopes have different identity granularity. Shared setup receives only project/workspace identity, not a personal/thread identity; a fresh runtime credential is applied before resume. The docs warn lifecycle scripts are trusted code and can copy credentials into snapshots if written unsafely. They also document up to **24 hours of delayed revocation** for workspace-scope token minting after membership removal, whereas project-scope minting checks current project access. [Secrets/OIDC](https://ampcode.com/docs/orbs/handling-secrets#oidc).

Portals are authenticated HTTP exposure, defaulting to thread viewers; temporary public access can be granted per port for 1 hour, 3 hours, 1 day or 7 days without granting thread/review access. Workspace policy can disable external sharing, and public-access controls are unavailable for self-hosted orbs. App authentication is separate. Inside the owning orb, requests to its own portal URL bypass Amp’s portal sign-in, so a successful agent browser test does **not** verify external-viewer authentication. Cross-portal browser calls are unsupported because CORS preflights lack portal authentication. [Portals](https://ampcode.com/docs/orbs/portals#access-and-lifecycle).

The customization cookbook documents outbound integration through Tailscale, including exit-node routing, and FUSE storage mounts for sharing data/increasing storage. These are explicit integrations with credential and identity implications, not evidence that arbitrary private networks are reachable by default. [Customization cookbook](https://ampcode.com/docs/orbs/customizing#connect-to-a-shared-network).

## 7. Availability, capacity and documented prices

### Standard compute rates

All current sizes list **60 GB disk**, with per-minute billing at the following standard hourly equivalents. Enterprise pricing is shown in project settings rather than guaranteed by this table. [Sizes and costs](https://ampcode.com/docs/orbs/sizes-and-costs).

| Size | CPU / RAM | Standard hourly equivalent |
| --- | --- | --- |
| `a1.tiny` | 1 / 2 GB | $0.08 |
| `a1.small` | 2 / 4 GB | $0.17 |
| `a1.medium` | 4 / 8 GB | $0.33 |
| `a1.large` | 8 / 16 GB | $0.66 |
| `a1.xxlarge` | 16 / 32 GB | $1.32 |
| `a1.3xlarge` | 16 / 44 GB | $2.13 |

The last size is restricted to Gigawatt subscribers and Enterprise workspace members. Default is `a1.medium`, except personal Megawatt projects default to `a1.small`; changing a project default does not resize an existing orb. Current pacing allows a burst of **20 metered orb starts per user**, then one new start every five minutes; excess starts wait with an estimate. This is a spawn-rate policy, not a documented limit of 20 simultaneously running agents. [Sizes](https://ampcode.com/docs/orbs/sizes-and-costs), [pacing](https://ampcode.com/docs/orbs#orb-spawn-pacing).

### Plans and billing caveats

The live pricing page currently advertises **Hobby free** with pay-as-you-go orbs and free own-machine runners; **Individual $20/month** with “45,000 minutes of orbin’ time”; **Teams no extra charge** with mixed member tiers and shared billing; and **Enterprise custom**. It still refers to Megawatt/Gigawatt elsewhere. Treat those as the published current labels, not a reconstructed complete legacy tier catalogue. [Live pricing](https://ampcode.com/pricing), [pricing docs](https://ampcode.com/docs/pricing).

The pricing docs say larger machines consume an allowance faster and smaller ones more slowly, but the fetched public text did not establish the normalization behind the advertised 45,000 minutes. Do not interpret it as 45,000 hours-equivalent minutes of any selected size. Model/tool usage and compute allowances are separate; BYOK/subscription inference is billed under that connection, not free cloud compute. Some supporting tools still consume Amp credits. Runners being free means no Amp runner compute charge, not free hardware/electricity or unlimited provider inference. [Pricing docs](https://ampcode.com/docs/pricing), [Dial costs](https://ampcode.com/docs/the-dial).

## 8. What belongs to which layer?

This table is **analysis based on the cited interfaces**, not Amp’s official component taxonomy.

| Layer | Responsibilities in the documented experience | Implication |
| --- | --- | --- |
| **Model** | Reasoning over supplied context, choosing actions, writing code or summaries; distinct models can serve main, Oracle and subagent roles. [Models](https://ampcode.com/docs/models-and-subagents) | A model alone does not provide machines, durable threads, permissions or wake schedules. |
| **Harness / agent runtime** | Prompts, tool definitions, tool execution loop, context/subagents, modes, custom agents and plugin hooks. The Dial combines model, effort, prompt, tools and Oracle. [Dial](https://ampcode.com/docs/the-dial), [plugin API](https://ampcode.com/docs/plugin-api) | Scoped Task workers are not interchangeable with independently addressable long-lived thread agents. |
| **Execution environment** | Filesystem, tools, services, terminals, machine resources and private-network access. Amp provisions these for managed orbs; users provide them for runners. [Customization](https://ampcode.com/docs/orbs/customizing), [runners](https://ampcode.com/docs/cli/runners) | Provisioning and trust differ even when the same thread UI controls both. |
| **Orchestration / collaboration product** | Thread identity/storage, clients, project selection, permissioned sharing, notifications, message/file delegation, schedules/webhooks, lifecycle/billing and result review/shipping. [Threads](https://ampcode.com/docs/threads), [agent-to-agent](https://ampcode.com/docs/orbs/agent-to-agent), [events](https://ampcode.com/docs/orbs/event-driven) | The valuable system is the coordinated workflow, not the VM label. |

**Important current correction:** Amp cannot accurately be described as having no model choice or only its own harness. Current docs allow per-role model/effort pins, custom agent modes, BYOK and linked subscriptions. **Claude Code mode** uses the Claude Agent SDK rather than Amp’s own agent and supports orbs, runners, sharing, multiplayer and thread messaging; it has explicit feature differences and runner installation/login requirements. That does not establish arbitrary support for every third-party CLI or a model-neutral universal orchestrator. [Dial, modified 2026-10-13](https://ampcode.com/docs/the-dial), [runners](https://ampcode.com/docs/cli/runners).

## 9. Implications for Kizn — analysis, not Amp facts or implementation authorization

### Product positioning

For Tommy’s central-machine workflow, the closest competitor is **runners plus cross-device threads**, not just managed orbs. For someone without a powerful machine, Amp’s managed compute already supplies a concrete alternative. A Kizn proposition limited to “agents on a remote machine controlled by phone” or “parallel agents” would overlap substantially with documented Amp capabilities. This assessment follows the deployment and coordination boundaries above; it is not evidence about adoption or product quality.

A more useful design question is whether Kizn should provide a common control layer across **existing machines, rented compute and heterogeneous harnesses**, while making host ownership, costs and authority visible. That is a potential product direction, not an approved design. Amp’s current runners and Claude Code mode mean multi-machine and multi-harness support cannot simply be assumed unique.

### Five germane design ideas

1. **Separate work identity, executor and viewing surface.** A stable task/thread should be addressable from multiple computers/phone without pretending the phone executes it. Retain explicit executor identity and liveness, including what happens when a user-owned host is off.
2. **Make assignment and transfer explicit.** Show whether delegation sends context only, files, commits or a complete checkout. Model child-result integration as owned work; do not imply a message automatically merges code.
3. **Expose trust before collaboration.** Distinguish viewing, prompting, shell access, secret use, host ownership and billing. An all-workspace trust switch is not equivalent to constrained multi-human permissions. Remote directories/worktrees are not security sandboxes.
4. **Design unattended lifecycle around truth.** Display running, sleeping, waiting, disconnected, failed and deleted states separately; make wake costs and schedule ownership legible. A webhook accepted/queued is not a completed action.
5. **Use durable knowledge as a complement, not a duplicate conversation store.** Kizn could connect project decisions and durable evidence to agent tasks across tools/machines, while the orchestration surface retains live work state. This is a hypothesis about complementary Kizn-branded products, not a claim of proven demand or an Amp knowledge-base omission.

### Limits of this comparison

This investigation does not establish that Amp lacks arbitrary-host migration, stricter contributor roles, heterogeneous external-tool integration, rich scheduling, or any unpublished enterprise feature. It establishes what the reviewed public sources describe, their explicit restrictions, and where evidence remains unavailable. No changes to Herdr-derived runtime scope or Git/Jujutsu workflows are authorized by this note.

## 10. Conflicting, gated and historical evidence

**June launch vs current manual.** The June 30 announcement points to `/manual/orbs`; requesting that old URL now resolves to `/docs/orbs`. Use the live documentation for current workflows and limits, not the older link structure as a second independent product version. The launch announcement is useful for dating introduction, not current billing terms. [Launch](https://ampcode.com/news/agents-in-orbs), [legacy URL](https://ampcode.com/manual/orbs?plain=1), [current overview](https://ampcode.com/docs/orbs).

**“No limit” vs paced starts.** The August 25 essay says one can spawn hundreds/thousands with “no limit.” The current overview documents a 20-start burst and one further start per five minutes. These do not necessarily conflict about total eventual thread count, but the essay cannot support unlimited immediate provisioning. [Essay](https://ampcode.com/notes/orbs-explained), [current pacing](https://ampcode.com/docs/orbs#orb-spawn-pacing).

**“Ephemeral” vs retained state.** Early descriptions and provider terminology call compute ephemeral; the current security reference explicitly retains paused filesystem and memory until thread deletion. Interpret “fresh per new thread” separately from “disposable or stateless after each turn.” [Launch](https://ampcode.com/news/agents-in-orbs), [security](https://ampcode.com/security#orb-data).

**Self-hosting language needs scope.** The security reference says Amp does not offer self-hosted deployments, while the indexed self-hosted-orb overview mentions company-cloud execution and the portal docs acknowledge self-hosted orbs. A plausible distinction is self-hosted execution versus self-hosted whole Amp control plane, but that is **inference**, not resolved by the gated page. Do not advertise either full self-hosting or its blanket impossibility from this evidence. [Security](https://ampcode.com/security#system-components), [gated overview](https://ampcode.com/docs/orbs/self-hosted), [portals](https://ampcode.com/docs/orbs/portals#access-and-lifecycle).

**Current documents can disagree internally.** Shared-runner owner-thread multiplayer eligibility differs between a sharing-section bullet and the multiplayer page/runner feature table. Group contribution controls also need a precise explanation alongside the whole-workspace multiplayer warning. Full current authorization semantics require confirmation, not an inferred least-privilege promise. [Runners](https://ampcode.com/docs/cli/runners), [multiplayer](https://ampcode.com/docs/collaborate/multiplayer), [threads](https://ampcode.com/docs/threads).

## 11. Verification ledger and open questions

### Verified from publicly readable primary text

Read current docs for orbs, setup, projects, customization, costs, secrets, portals, threads, runners, multiplayer, native apps, models/modes, shipping, agent-to-agent work, schedules, webhook delivery and generated plugin API types. Read the dated June launch and August explanation. Confirmed the legacy manual redirects, the self-hosted HTML sign-in redirect, and the self-hosted Markdown HTTP 401. Prices and restrictions here are a dated public-document snapshot, not account-specific quotes.

### Not verified

No live provisioning, pause/wake, permissions, sync safety, CLI command behavior, mobile install, notifications, billing, agent quality or security isolation was exercised. No privileged API was called, OIDC token minted, webhook registered, repository cloned into Amp or private enterprise material read. Public API type declarations were inspected, not Amp’s private implementation source. Search-index text for self-hosted orbs is weaker than an accessible full page and is labelled accordingly.

### Questions worth resolving before procurement or product commitments

1. **Hosting:** What are self-hosted-orb eligibility, provisioning ownership, providers/regions, network policy, retention and pricing? Is only execution self-hosted? What are the precise isolation and recovery guarantees?
2. **Authority:** Are contributors ever restricted to selected groups rather than the whole workspace? Which owner threads qualify on shared runners? Can ownership/billing be transferred, and what is the exact revocation latency for each form of access?
3. **Reliability:** What survives executor loss versus normal pause? Is there supported migration between executors? What are message/file delivery guarantees for agent-to-agent work, cancellation propagation and orphan cleanup? Webhook guarantees do not answer these questions for all coordination paths.
4. **Cost/capacity:** What size-normalized units underlie “45,000 minutes,” which current tiers/allowances apply, what quotas beyond spawn pacing apply, and what is guaranteed availability for requested sizes?
5. **Interoperability:** Beyond the documented Claude Code mode and plugin APIs, what external harnesses can participate with equivalent steering, artifact handling, observability and human permissions? What precisely does `amp sync` do with conflicting local edits?

## Source index

Dates below are page `article:modified_time` metadata observed on the research date unless marked as publication dates. Metadata dates are not proof every paragraph was revised then. Undated pages were read on 2026-10-14.

### Deployment and environment

| Source | Observed date | Why it matters |
| --- | --- | --- |
| [Orbs overview](https://ampcode.com/docs/orbs) | 2026-09-14 | Isolation, remote controls, uploads, sync, spawn pacing |
| [Getting started](https://ampcode.com/docs/orbs/getting-started) | 2026-09-01 | Actual web provisioning and project preparation flow |
| [Customizing orbs](https://ampcode.com/docs/orbs/customizing) | 2026-09-29 | Debian/tools, snapshots, hooks, networking/storage |
| [Runners](https://ampcode.com/docs/cli/runners) | 2026-10-10 | BYO machine, worktrees, shared runners, explicit non-sandbox boundary |
| [Self-hosted overview](https://ampcode.com/docs/orbs/self-hosted) | Unavailable; sign-in redirect | Official indexed BYO-cloud statement; full scope not verified |

### Workflow and humans

| Source | Observed date | Why it matters |
| --- | --- | --- |
| [Threads](https://ampcode.com/docs/threads) | 2026-10-12 | Steering, handoffs, identity, exports, viewing/contribution |
| [Spawning orbs](https://ampcode.com/docs/cli/spawning-orbs) | 2026-09-17 | CLI launch/continue/stream behavior and limitations |
| [Multiplayer](https://ampcode.com/docs/collaborate/multiplayer) | 2026-10-05 | Human contribution, sharing duration, secrets/host access |
| [macOS and iOS](https://ampcode.com/docs/macos-and-ios) | 2026-09-27 | Same threads, notification flow, beta availability |
| [Shipping](https://ampcode.com/docs/orbs/shipping) | 2026-10-07 | Prompt-driven ship/rebase/test/push and custom alternatives |

### Coordination and harness

| Source | Observed date | Why it matters |
| --- | --- | --- |
| [Agent to agent](https://ampcode.com/docs/orbs/agent-to-agent) | 2026-08-25 | Communicating independent threads, explicit file transfers |
| [Models and subagents](https://ampcode.com/docs/models-and-subagents) | 2026-10-13 | Specialist workers and their communication/intervention limits |
| [The Dial](https://ampcode.com/docs/the-dial) | 2026-10-13 | Model/role tuning, Claude Agent SDK mode, costs |
| [Plugin API](https://ampcode.com/docs/plugin-api) | Undated | Generated public types and coordination tools |
| [Automations](https://ampcode.com/docs/orbs/automations) / [Event-driven orbs](https://ampcode.com/docs/orbs/event-driven) | 2026-08-28 / 2026-09-25 | Schedule ownership, wake, webhook guarantees and constraints |

### Policy, money and history

| Source | Observed date | Why it matters |
| --- | --- | --- |
| [Projects](https://ampcode.com/docs/projects) | 2026-09-28 | Code ownership, repository setup, settings permissions |
| [Secrets](https://ampcode.com/docs/orbs/handling-secrets) / [Portals](https://ampcode.com/docs/orbs/portals) | 2026-09-30 / 2026-10-14 | OIDC/revocation, shared snapshot identity, network exposure |
| [Sizes and costs](https://ampcode.com/docs/orbs/sizes-and-costs) / [Pricing docs](https://ampcode.com/docs/pricing) / [Live pricing](https://ampcode.com/pricing) | 2026-09-24 / 2026-09-14 / undated | Compute table, idle billing, current public plan claims |
| [Security reference](https://ampcode.com/security) | Undated | Provider, retention/deletion, encryption, ownership and system boundaries |
| [Agents in Orbs](https://ampcode.com/news/agents-in-orbs) / [Orbs, Explained](https://ampcode.com/notes/orbs-explained) | Published 2026-06-30 / 2026-08-25 | Introduction and terminology; not authority over newer restrictions |
