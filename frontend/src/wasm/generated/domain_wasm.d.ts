/* tslint:disable */
/* eslint-disable */

export interface TimezoneOffsets {
    /** The zone's UTC offset in seconds at `epochMinutes` minutes after the Unix epoch. */
    offsetSecondsAt(epochMinutes: number): number
}



export type FileChangeAction = "ignore" | "warn" | "fatal"

export interface FileChangePatternRow {
    path: string
    action: FileChangeAction
}



/**
 * See [`domain::hooks::MAX_HOOK_COMMAND_TIMEOUT_SECONDS`].
 */
export function maxHookCommandTimeoutSeconds(): number;

/**
 * The next `count` runs of `expression` after `from` (RFC 3339) in the zone
 * named `timezone`, computed by the scheduler's own
 * [`domain::schedule::next_runs_in`] so DST gaps and repeats resolve exactly
 * as the schedule will fire. Each run is an RFC 3339 UTC timestamp.
 *
 * # Errors
 *
 * Fails if `from` is not RFC 3339, `offsets` cannot answer for the zone, or
 * the expression is invalid.
 */
export function nextCronRuns(expression: string, from: string, timezone: string, offsets: TimezoneOffsets, count: number): string[];

/**
 * The default title, body and web-push body templates, in that order; see
 * [`template::DEFAULT_TITLE_TEMPLATE`], [`template::DEFAULT_BODY_TEMPLATE`]
 * and [`template::DEFAULT_PUSH_BODY_TEMPLATE`].
 *
 * Returned as one `Array` rather than a `Vec<String>`: wasm-bindgen emits
 * identical glue for exports with identical signatures, and
 * [`notification_template_placeholder_keys`] already returns `Vec<String>`.
 */
export function notificationTemplateDefaults(): [title: string, body: string, pushBody: string];

/**
 * Every `{{placeholder}}` key the renderer understands; see
 * [`template::placeholder_keys`].
 */
export function notificationTemplatePlaceholderKeys(): string[];

/**
 * Parses the raw textarea form into `[path, action]` pairs; see
 * [`grammar::parse_file_change_patterns`].
 */
export function parseFileChangePatterns(raw: string): Array<[path: string, action: FileChangeAction]>;

/**
 * Renders a notification template against a JSON payload exactly as a channel
 * delivers it; see [`template::render_template`].
 *
 * # Errors
 *
 * Fails if `payload_json` is not valid JSON.
 */
export function renderNotificationTemplate(template: string, payload_json: string): string;

/**
 * Serializes rows, given as parallel `paths` and `actions` columns, back into
 * the raw textarea form; see [`grammar::serialize_file_change_patterns`].
 *
 * Returns a `JsString` rather than a `String`: wasm-bindgen emits identical
 * glue for exports with identical signatures, and the notification renderer
 * already returns `Result<String, JsError>`.
 *
 * # Errors
 *
 * Fails if the columns differ in length or an action is not a known keyword.
 */
export function serializeFileChangePatternColumns(paths: string[], actions: FileChangeAction[]): string;

/**
 * Validates a cron expression exactly as the server does when a schedule is
 * saved; see [`domain::schedule::validate_cron`].
 *
 * Returns the server's error message, or `undefined` when the expression is valid.
 */
export function validateCron(expression: string): string | undefined;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly maxHookCommandTimeoutSeconds: () => number;
    readonly nextCronRuns: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => void;
    readonly notificationTemplateDefaults: () => number;
    readonly notificationTemplatePlaceholderKeys: (a: number) => void;
    readonly parseFileChangePatterns: (a: number, b: number, c: number) => void;
    readonly renderNotificationTemplate: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly serializeFileChangePatternColumns: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly validateCron: (a: number, b: number, c: number) => void;
    readonly __wbindgen_export: (a: number, b: number) => number;
    readonly __wbindgen_export2: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_export3: (a: number) => void;
    readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
    readonly __wbindgen_export4: (a: number, b: number, c: number) => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
