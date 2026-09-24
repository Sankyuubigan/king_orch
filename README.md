# King Orch 👑

**King Orch** is a standalone desktop application for local, private, and restriction-free communication with Large Language Models (LLMs) through a multi-agent system.

## 🌟 Core Philosophy

- **Local-first:** Run your `.gguf` models on your own hardware. The application uses a local `llama-server.exe` process and does not require an external LLM provider.
- **Explicit control:** No API keys are required for local inference, and the user controls the model, context, and agent workflow.
- **File-native agents:** Agents and workflows are ordinary `.md` and YAML files that can be inspected, edited, versioned, and shared.

## 📖 Why was this project created?

King Orch started as an environment for building and testing an AI psychotherapist agent with user-defined rules and logic. It evolved into a universal local studio for storing, composing, and running multi-agent workflows.

## ⚙️ Features

- **Multi-agent orchestration:** Agents and workflows can delegate work to other agents and tools.
- **Shared session state:** The session `messages[]` array is the source of truth, while agent results are stored as messages with their namespace and author.
- **Tool integration:** Built-in tools and MCP integrations can support web search, file operations, local RAG, coding tools, and external services.
- **YAML workflow graph:** Routing, branching, conditions, and node sequencing are defined in visual YAML graphs.
- **Local model management:** The llama-engine plugin manages local GGUF models, model metadata, backend installation, and model selection.

## Agent Modes

The current architecture has two entry-point modes and structural agent roles. The legacy `primary`, `router`, and `worker` frontmatter modes are not separate execution modes.

### Workflow mode

A visible YAML workflow is the entry point. The workflow engine executes graph nodes such as LLM workers, fact extractors, switches, conditions, and sub-workflows. Routing is controlled by the graph rather than by instructions hidden in an agent prompt.

### Direct Markdown agent mode

When no matching visible YAML workflow exists, King Orch runs the matching visible Markdown agent through the legacy orchestrator. The `.md` file contains the agent's business and communication logic, while the orchestrator handles the execution loop.

### Agent roles

- **Backend agents** perform specialist work inside a workflow and may be followed by outgoing graph edges.
- **Frontend agents** produce user-facing output and terminate the workflow run. The next run starts from the updated session state after the user replies.
- **Built-in workers and routers** are workflow nodes implemented by the graph engine, not obsolete frontmatter mode flags.

## 🧩 Advantages of YAML Workflow Architecture

1. **Visual process control:** The graph makes the execution path, branching conditions, agent sequence, and terminal nodes easy to inspect, control, and change visually.
2. **Logic outside prompts:** Routing and orchestration live in YAML and the workflow engine, so local LLM prompts stay focused on agent business logic instead of repeatedly reasoning about control flow.

## 🏗 Architecture overview

- **Frontend:** Controllers coordinate behavior, services encapsulate Tauri IPC, UI components render DOM, and the Store holds shared application state.
- **Backend:** The API layer exposes Tauri commands, the domain layer owns orchestration and business rules, and the infrastructure layer owns sessions, configuration, MCP, networking, and LLM integration.
- **Workflow separation:** YAML files define routing and graph execution; Markdown files define agent business logic and presentation rules.
- **Session model:** `messages[]` in the session JSON file is the single source of truth for conversation history and agent results.
- **Local inference:** The application starts a separate `llama-server.exe` process and communicates with it over localhost HTTP. The LLM engine is not linked into the application binary.

## 🚀 Getting Started

1. Download a compatible `.gguf` model, such as a Llama, Gemma, or ChatML-based model.
2. Open King Orch and add the model through the local models panel.
3. Select the model and a visible workflow or Markdown agent from the interface.
4. Start chatting.

For agent creation and workflow authoring, see [`docs/AGENT_CREATION_GUIDE.md`](docs/AGENT_CREATION_GUIDE.md).
