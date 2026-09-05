# Two-Phase Generation Research: Findings

## Executive Summary

Three major open-source projects implement "two-phase generation" (thinking + structured output in one request) using **fundamentally different approaches**, none of which are truly "one inference call" in the purest sense. The key finding: **no project achieves true single-request two-phase generation without either a composite grammar or a two-pass mechanism**. Each has tradeoffs.

---

## 1. llama.cpp: Composite GBNF Grammar (True Single Request)

### Approach: Grammar that wraps thinking + structured output

llama.cpp **does** support thinking + grammar in a single inference call, using a **composite GBNF grammar** that wraps both phases.

### Mechanism

The grammar explicitly handles the `<think>` / `</think>` token boundary:

```
# From grammars/README.md — tokens section
# Match a thinking block: <think>...</think>
root ::= <think> thinking </think> json_after_think
thinking ::= !</think>*
json_after_think ::= <JSON grammar here>
```

**Key file:** `grammars/README.md` — Tokens section documents `<think>` / `</think>` as token-level grammar elements.

### Grammar Token Matching

llama.cpp grammars support matching **special tokens** by string or ID:
- `<token-string>` — matches exact token (e.g., `<think>`)
- `<[token-id]>` — matches token by ID
- Negation: `!<think>` matches any token EXCEPT `<think>`

This means you can build a **composite grammar**:
```
root ::= think_block json_output
think_block ::= <think> [^<]* | <think> (!</think>)* </think>
json_output ::= <json_schema_grammar>
```

### `reasoning-format` Parameter

The server has `--reasoning-format` with values:
- `none`: thoughts stay in `message.content`
- `deepseek`: thoughts go to `message.reasoning_content`
- `deepseek-legacy`: `<think>` tags in content + `reasoning_content`
- `auto`: detected from template

**Source:** `tools/server/README.md` lines ~1260-1270

### `grammar_lazy` + `grammar_triggers` (The "Lazy Grammar" Mechanism)

llama.cpp has an **explicit lazy grammar mechanism**:
- `grammar_lazy: true` — grammar is NOT applied from the start
- `grammar_triggers: [{"word": "<tool_call>", "at_start": false}]` — grammar activates when trigger token appears

This is **exactly** the "grammar trigger" pattern you asked about:
1. Phase 1: Model generates freely (no grammar)
2. When trigger token appears → grammar activates
3. Phase 2: Model generates constrained output

**Source:** Issue #22537 documents the API: `grammar`, `grammar_lazy`, `grammar_triggers` parameters.

### Known Bugs (as of b330, Aug 2026)

**Issue #22537** (closed as wontfix): "grammar not applied to content when thinking is enabled — lazy trigger approach also broken"
- Raw grammar leaks into `reasoning_content` — model generates garbage during thinking
- `grammar_lazy` + `grammar_triggers` on `<think>` fails — the `>` character blocks the model from closing thinking
- `--reasoning-budget 0` crashes the server

**Issue #27217** (open): "tool_choice: required accepted but not enforced on templates with supports_preserve_reasoning=true"
- Grammar is built and marked "eager", tool calls are non-optional — yet generation is unconstrained
- Specifically affects templates with `supports_preserve_reasoning: true`

**Issue #24807** (closed): "lazy grammar fails to prevent malformed tool-call XML"
- The lazy grammar trigger fires on `<tool_call>` but occasionally fails to constrain output

### Auto-Parser (chat-auto-parser-generator.cpp)

**Key code (from issue #27217):**
```cpp
// ~line 84
bool include_grammar = has_response_format || (has_tools &&
        ((inputs.tool_choice == COMMON_CHAT_TOOL_CHOICE_AUTO && !trigger_marker.empty()) ||
          inputs.tool_choice == COMMON_CHAT_TOOL_CHOICE_REQUIRED));

// ~line 89 — REQUIRED yields NON-lazy (eager) grammar
data.grammar_lazy = !has_response_format && inputs.tool_choice == COMMON_CHAT_TOOL_CHOICE_AUTO;

// ~line 369 and ~line 500 — for REQUIRED the tool calls are NOT optional
if (!require_calls) { tool_calls = p.optional(tool_calls); }
```

### Summary for llama.cpp

| Feature | Status |
|---------|--------|
| Grammar + thinking (composite) | ✅ Supported via GBNF token matching |
| Lazy grammar (trigger-based) | ✅ Supported but buggy |
| reasoning-format + grammar | ⚠️ Partial — grammar can leak into reasoning_content |
| JSON schema + thinking | ⚠️ Same issues as grammar |
| Single inference call | ✅ Yes — composite grammar runs in one pass |

---

## 2. vLLM: Two-Phase via Structural Tags or Native Reasoning Parser

### Approach: Reasoning parser + structured outputs

vLLM supports reasoning outputs with structured outputs **natively** through its reasoning parser system.

### Mechanism

From `docs/features/structured_outputs.html`:

> You can also use structured outputs with Reasoning Outputs for reasoning models.
> 
> ```python
> vllm serve deepseek-ai/DeepSeek-R1-Distill-Qwen-7B --reasoning-parser deepseek_r1
> ```
> 
> Note that you can use reasoning with any provided structured outputs feature.

vLLM separates reasoning from content at the **parser level**:
1. The reasoning parser extracts `<think>...</think>` into `reasoning` field
2. The remaining content is constrained by structured output grammar
3. **Both happen in one inference call** — the grammar constrains only the non-reasoning part

### `enable_in_reasoning` Flag

> When using Qwen3 Coder models with reasoning enabled, structured outputs might become disabled if the reasoning content does not get parsed into the `reasoning` field separately. To use both features together, add: `--structured-outputs-config.enable_in_reasoning=True`

### `structural_tag` Feature

vLLM also supports `structural_tag` — constraining output within specific tags:
```python
extra_body={"structured_outputs": {"structural_tag": ...}}
```

This is designed for cases where reasoning happens in one region and structured output in another.

### Summary for vLLM

| Feature | Status |
|---------|--------|
| Reasoning + structured output | ✅ Supported natively |
| Single inference call | ✅ Yes — parser separates reasoning/content |
| enable_in_reasoning | ✅ Explicit flag for combined mode |
| Backend support | xgrammar or guidance |

---

## 3. Ollama: Double-Request Pattern (NOT Single Request)

### Approach: Two sequential requests

Ollama does **NOT** support thinking + structured output in a single request. Instead, it uses a **double-request pattern**:

### Mechanism (from PR #14288 and #12460)

1. **Request 1**: Run WITHOUT format constraint → model thinks freely
2. **Detect thinking completion** → cancel request when content starts
3. **Rebuild prompt** with thinking content appended as assistant message
4. **Request 2**: Run WITH format constraint → produces structured output

**Source:** PR #14288 commit message:
> "When a format constraint is set and the model has thinking capability, the first completion request runs without the format constraint so the model can think freely. Once the parser detects that thinking is done and content is starting, the request is cancelled. The prompt is rebuilt with the thinking content appended as an assistant message... A second completion request runs with the format constraint active."

### Current Status

- **`/api/chat`**: Double-request pattern works (implemented in PR #12460)
- **`/api/generate`**: NOT yet merged (PR #14288 still open as of Sep 2026)
- MaxusAI fork has a working implementation (PR #31 in MaxusAI/ollama fork)

### Why Not Single Request?

Ollama uses llama.cpp as its backend but **does not expose the composite grammar mechanism**. It applies the format grammar from the start of generation, which breaks thinking models because the grammar constrains the thinking phase too.

### Summary for Ollama

| Feature | Status |
|---------|--------|
| Thinking + structured output | ⚠️ Double-request only |
| Single inference call | ❌ No — requires two requests |
| /api/chat support | ✅ Working (double-request) |
| /api/generate support | ❌ Not merged upstream |

---

## 4. Architectural Comparison

| Aspect | llama.cpp | vLLM | Ollama |
|--------|-----------|------|--------|
| **Mechanism** | Composite GBNF grammar | Reasoning parser + structured output | Double-request |
| **Single request** | ✅ Yes | ✅ Yes | ❌ No |
| **Grammar-level** | Grammar wraps think + JSON | Parser separates reasoning, grammar on content | Grammar on all output (breaks thinking) |
| **Lazy grammar** | ✅ `grammar_lazy` + `grammar_triggers` | N/A | N/A |
| **Known bugs** | Grammar leaks into reasoning_content on some models | Reasoning must be parsed first (`enable_in_reasoning`) | Thinking + format broken on /api/generate |
| **Maturity** | Active development, several open bugs | Stable, production-ready | Working for chat, broken for generate |

---

## 5. For King Orch Project: Key Takeaways

### What We Already Have (hybrid GBNF, Method 3)

Our approach in King Orch (`root ::= think-block json-object`) is **architecturally identical** to what llama.cpp supports natively:

```
root ::= think-block json-object
think-block ::= "<think>" [^<]* "</think>" | ""
```

This is the **composite grammar pattern** — the same approach used by llama.cpp's GBNF token matching.

### What We Need to Know

1. **Our `disable_reasoning: false`** for hybrid GBNF agents is correct — the grammar engine needs reasoning enabled to generate `<think>` blocks
2. **The grammar constrains the JSON part** — `<think>...</think>` is consumed by the grammar as a prefix, then the JSON grammar takes over
3. **Known limitation**: Some models (especially Qwen3.6) have issues with grammar + thinking — the grammar can "leak" into reasoning content
4. **The lazy grammar approach** (`grammar_lazy` + `grammar_triggers`) exists in llama.cpp but is buggy — our composite grammar approach is more reliable

### Our Approach vs Others

| Project | Approach | Single Request? |
|---------|----------|-----------------|
| **King Orch** | Composite GBNF (`think-block json-object`) | ✅ Yes |
| **llama.cpp** | Same composite GBNF | ✅ Yes |
| **vLLM** | Reasoning parser + structured output | ✅ Yes |
| **Ollama** | Double-request | ❌ No |

**Our approach is correct and matches the state-of-the-art.** The composite grammar pattern is the standard way to achieve two-phase generation in a single inference call.
