# King Orch 👑

**King Orch** is a standalone desktop application designed for local, private, and restriction-free communication with Large Language Models (LLMs) through a powerful multi-agent system.

## 🌟 Core Philosophy
- **100% Local & Private:** Run your favorite `.gguf` models directly on your hardware. No internet connection to external LLM providers is needed.
- **Zero Restrictions:** No artificial guardrails, no system dependencies, and **no API keys** required. You are in full control of your AI.
- **Markdown-Native Agents:** Agents are simply defined as `.md` files. No complex databases or proprietary formats. Just pure text that you can easily version control, edit, and share.

## 📖 Why was this project created?
I originally started developing this project specifically as an engine to create and thoroughly test an AI psychotherapist agent. I needed a flexible, local environment where I could build and refine the agent according to my own rules and logic.

During my research, I realized there was absolutely no free, accessible, and comfortable software available that allowed developers to easily store, orchestrate, and use agents as simple Markdown files. King Orch was built to fill that gap, evolving into a universal multi-agent studio.

## ⚙️ Features
- **Multi-Agent Orchestration:** Agents can seamlessly call other subagents to delegate tasks. 
- **Shared Session State:** The system automatically maintains a "Dossier" (Session State) passing context between specialist agents without cluttering the main prompt.
- **Tool Integration (MCP):** Built-in tools for Web Searching (DuckDuckGo), File System operations, Local RAG, and YouTube summarization.
- **Multiple Agent Modes:** 
  - `primary` (User-facing communicators)
  - `router` (Logical orchestrators that decide the pipeline path)
  - `worker` (Specialists that execute specific tasks and analyze data)

## 🚀 Getting Started
1. Download a compatible `.gguf` model (e.g., Llama 3, Gemma, ChatML-based models).
2. Open King Orch, add your model via the UI, and select it.
3. Choose your Primary Agent from the dropdown and start chatting!

*(To learn how to create your own agents, please refer to the `AGENT_CREATION_GUIDE.md` file).*