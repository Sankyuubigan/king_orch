/**
 * Строгие типы вместо any.
 * ИИ-агенты работают в 10 раз точнее, когда видят конкретные интерфейсы,
 * а не загадочный `any`, под которым может скрываться что угодно.
 */

export interface ChatMessage {
    role: 'user' | 'assistant' | 'system' | 'thought';
    content: string;
    sub_calls?: SubCall[];
    agent_name?: string;
}

export interface SubCall {
    agent_name: string;
    prompt: string;
    response: string;
    time_sec: number;
    tool_calls: ToolCallInfo[];
}

export interface ToolCallInfo {
    tool_name: string;
    arguments: string;
    result: string;
}

export interface ModelParams {
    temperature: number;
    top_k: number;
    top_p: number;
    min_p: number;
    repetition_penalty: number;
    presence_penalty: number;
}

export interface AgentProfile {
    id: string;
    name: string;
    description: string;
    is_hidden: boolean;
    mode: 'primary' | 'router' | 'worker' | 'auto';
}

export interface ChatResponse {
    text: string;
    sub_calls: SubCall[];
    dossier: Record<string, string>;
}

export interface SessionMeta {
    id: string;
    title: string;
    updated_at: number;
    created_at: number;
}