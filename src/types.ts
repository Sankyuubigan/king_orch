export interface ChatMessage {
    id?: string;
    type: 'message' | 'thought';
    content: string;
    sub_calls?: SubCall[];
    author?: string;
    model?: string;
    time_sec?: number;
    attachments?: Attachment[];
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
    dry_multiplier: number;
    dry_base: number;
    dry_allowed_length: number;
    dry_penalty_last_n: number;
    xtc_probability: number;
    xtc_threshold: number;
}

export interface AgentEntry {
    id: string;
    name: string;
    description: string;
    entry_type: 'agent' | 'workflow';
    is_hidden: boolean;
    folder?: string;
}

export interface ChatResponse {
    text: string;
    sub_calls: SubCall[];
    messages: ChatMessage[];
}

export interface SessionMeta {
    id: string;
    title: string;
    updated_at: number;
    created_at: number;
}

export interface ThoughtMenuCallbacks {
    onDeleteThoughts: (assistantUid: string | null, thoughtUids: string[]) => void;
    onCloneFromThoughts: (assistantUid: string) => void;
}

export interface TestCaseDef {
    input_data: string;
    right_answer_context: string;
}

export interface SingleTestResult {
    input_data: string;
    right_answer_context: string;
    responses: Record<string, string>;
}

export interface PipelineTestInfo {
    id: string;
    workflow_name: string;
    model_path: string;
    source_file: string;
    timeout_sec: number;
}

export interface LevelResult {
    passed: boolean;
    details: string[];
}

export interface PipelineTestResult {
    test_id: string;
    model_name: string;
    workflow_name: string;
    duration_ms: number;
    level1_structure: LevelResult;
    level2_file: LevelResult;
    level3_functional: LevelResult;
    overall_passed: boolean;
    messages: { author: string; content_preview: string; msg_type: string }[];
    report_path: string | null;
}

export interface Attachment {
    file_name: string;
    mime_type: string;
    data_base64: string;
}

export interface CatalogEntry {
    name: string;
    download_url: string;
    mmproj_url?: string;
    tokenizer_id?: string;
    uncen?: boolean;
    vision?: boolean;
    audio?: boolean;
}