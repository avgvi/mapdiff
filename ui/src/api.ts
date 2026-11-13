// The shapes the local server sends, and the calls that fetch them.
// They mirror crates/mapdiff-core/src/changes.rs and crates/mapdiff-server/src.

export type ScopeKind =
  | 'directory'
  | 'file'
  | 'document'
  | 'module'
  | 'class'
  | 'interface'
  | 'enum'
  | 'variant'
  | 'type_alias'
  | 'impl'
  | 'function'
  | 'method'
  | 'constructor'
  | 'field'
  | 'constant'
  | 'macro'
  | 'import'
  | 'block'

export type ChangeState =
  | 'added'
  | 'removed'
  | 'modified'
  | 'renamed'
  | 'moved_in'
  | 'moved_out'

export type ChangeKind = 'signature' | 'body' | 'comments' | 'formatting' | 'access'

export type Access = 'private' | 'restricted' | 'public'

export interface LineCount {
  added: number
  removed: number
}

export interface Change {
  state: ChangeState
  kinds: ChangeKind[]
  kind: ScopeKind
  lang_kind: string
  label: string
  old_label: string | null
  name: string | null
  access: Access
  old_id: string | null
  new_id: string | null
  /** For moves: the ID of the same scope on the other side. */
  counterpart: string | null
  old_path: string | null
  new_path: string | null
  /** Lines changed in the scope itself, not in scopes inside it. */
  own: LineCount
  /** Own lines plus everything inside. */
  total: LineCount
  /** For a moved scope: lines that differ between its old and new text. */
  moved_delta: LineCount | null
  /** Stays the same only while both sides of the scope stay the same. */
  fingerprint: string
  parent: number | null
  children: number[]
}

export interface Revision {
  label: string
  commit: string | null
}

export interface ChangeSet {
  schema_version: number
  base: Revision
  head: Revision
  ignore_whitespace: boolean
  stats: { files: number; scopes: number; added: number; removed: number }
  changes: Change[]
}

export interface Commit {
  hash: string
  short: string
  subject: string
  author: string
  date: string
}

export interface Meta {
  name: string
  root: string
  branch: string | null
  default_branch: string | null
  branches: string[]
  commits: Commit[]
}

export interface RowSpan {
  text: string
  /** A syntax class such as `keyword` or `string`. */
  class: string | null
  /** True for the words that differ from the other side. */
  changed: boolean
}

export interface Row {
  tag: 'equal' | 'delete' | 'insert'
  old_line: number | null
  new_line: number | null
  spans: RowSpan[]
}

export interface Location {
  path: string
  first_line: number
  last_line: number
}

export interface ChangeDetail {
  index: number
  id: string
  old: Location | null
  new: Location | null
  rows: Row[]
  /** Set when there is nothing to show, saying why. */
  note: string | null
}

/** What the reviewer is looking at. */
export type Target =
  | { mode: 'working' }
  | { mode: 'staged' }
  | { mode: 'branch'; base: string }
  | { mode: 'commit'; commit: string }
  | { mode: 'range'; spec: string }

/** The server's query parameters for a target. */
export function targetParams(target: Target): URLSearchParams {
  const params = new URLSearchParams()
  switch (target.mode) {
    case 'working':
      break
    case 'staged':
      params.set('staged', 'true')
      break
    case 'branch':
      // Three dots: from where this branch left the base.
      params.set('spec', `${target.base}...HEAD`)
      break
    case 'commit':
      params.set('commit', target.commit)
      break
    case 'range':
      params.set('spec', target.spec)
      break
  }
  return params
}

/** An error the server explained in a sentence. */
export class ApiError extends Error {
  readonly status: number

  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  let response: Response
  try {
    response = await fetch(path, init)
  } catch {
    throw new ApiError(0, 'The reviewer server is not answering. Is `mapdiff serve` still running?')
  }
  if (!response.ok) {
    let message = `The server answered ${response.status}.`
    try {
      const body: unknown = await response.json()
      if (typeof body === 'object' && body !== null && 'error' in body) {
        message = String(body.error)
      }
    } catch {
      // Not JSON: keep the generic message.
    }
    throw new ApiError(response.status, message)
  }
  return (await response.json()) as T
}

export function fetchMeta(): Promise<Meta> {
  return request('/api/meta')
}

export function fetchChanges(target: Target, signal?: AbortSignal): Promise<ChangeSet> {
  return request(`/api/diff?${targetParams(target)}`, { signal })
}

export function fetchDetail(
  target: Target,
  index: number,
  id: string,
  signal?: AbortSignal,
): Promise<ChangeDetail> {
  const params = targetParams(target)
  params.set('index', String(index))
  params.set('id', id)
  return request(`/api/change?${params}`, { signal })
}

export async function fetchReviewed(): Promise<string[]> {
  const body = await request<{ keys: string[] }>('/api/reviewed')
  return body.keys
}

export async function saveReviewed(keys: string[], reviewed: boolean): Promise<string[]> {
  const body = await request<{ keys: string[] }>('/api/reviewed', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ keys, reviewed }),
  })
  return body.keys
}

/**
 * The query for a route of the map: with a target, about that comparison;
 * without one, about the files as they are.
 */
export function mapParams(target: Target | null, more: Record<string, string> = {}): URLSearchParams {
  const params = target ? targetParams(target) : new URLSearchParams()
  if (target) params.set('compare', 'true')
  for (const [name, value] of Object.entries(more)) params.set(name, value)
  return params
}

/** What a map draws at a level. With a target, the map of that change. */
export function fetchMap(level: string, target: Target | null): Promise<import('./layout/types.ts').MapData> {
  return request(`/api/map?${mapParams(target, { level })}`)
}

/** How references are found for what the map shows. */
export interface References {
  /** From names only, from an index for every project, or for some. */
  by: 'name' | 'type' | 'mixed'
  /** Whether making an index could add something. */
  can_index: boolean
  /** Set while the indexers are running. */
  indexing: { step: string; seconds: number } | null
  /** Why the last run left a project without an index. */
  notes: string[]
}

export function fetchReferences(target: Target | null): Promise<References> {
  return request(`/api/references?${mapParams(target)}`)
}

/** Starts the language indexers in the background. */
export function startIndexing(): Promise<{ started: boolean }> {
  return request('/api/index', { method: 'POST' })
}

export interface PullRequest {
  number: number
  title: string | null
  base_branch: string
  /** The range that shows what it changes. */
  spec: string
}

/** Fetches a GitHub pull request from `origin`, so that it can be shown. */
export function fetchPullRequest(number: number): Promise<PullRequest> {
  return request('/api/pr', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ number }),
  })
}

/** One place where a name is used. */
export interface Place {
  path: string
  /** Counted from 1. */
  line: number
  /** Counted from 0, in characters of the whole line. */
  column: number
  end_column: number
  kind: string
  /** The line, without its indentation. */
  text: string
  indent: number
}

/** Another scope that uses a scope, or that it uses. */
export interface Use {
  id: string
  /** The box it is drawn in at the level asked for. */
  at: string
  kind: ScopeKind
  lang_kind: string
  label: string
  path: string
  count: number
  places: Place[]
}

export interface NodeInfo {
  /** `base` for a scope that is only there before the change. */
  side: 'head' | 'base'
  id: string
  kind: ScopeKind
  lang_kind: string
  label: string
  signature: string | null
  access: Access
  path: string
  first_line: number
  last_line: number
  ancestors: { id: string; label: string }[]
  callers: Use[]
  dependencies: Use[]
}

export function fetchNode(id: string, level: string, target: Target | null = null): Promise<NodeInfo> {
  return request(`/api/node?${mapParams(target, { id, level })}`)
}

export interface CodeSpan {
  text: string
  class: string | null
}

/** A file as highlighted lines. */
export async function fetchCode(
  path: string,
  target: Target | null = null,
  side: 'head' | 'base' = 'head',
): Promise<CodeSpan[][]> {
  const body = await request<{ lines: CodeSpan[][] }>(`/api/code?${mapParams(target, { path, side })}`)
  return body.lines
}

export interface Hit {
  id: string
  /** The box it is drawn in at the code level. */
  at: string
  /** The first level, from the top, that shows that box. */
  level: import('./layout/types.ts').Level
  kind: ScopeKind
  lang_kind: string
  label: string
  path: string
}

export async function fetchSearch(q: string): Promise<Hit[]> {
  const body = await request<{ hits: Hit[] }>(`/api/search?${new URLSearchParams({ q })}`)
  return body.hits
}
