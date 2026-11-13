// The changes of a comparison, beside the map, in an order to read them
// in: what is used before what uses it, one file at a time.

import type { Change } from '../api'
import { reviewKey } from '../model'
import { toneOf, type Diff } from './overlay.ts'

interface Props {
  diff: Diff
  /** The positions to list, in the order to read them. */
  order: number[]
  reviewed: Set<string>
  current: number | null
  callerFirst: boolean
  onFlip(): void
  onPick(position: number): void
  onReview(position: number, done: boolean): void
}

const WHAT: Record<Change['state'], string> = {
  added: 'added',
  removed: 'removed',
  modified: 'modified',
  renamed: 'renamed',
  moved_in: 'moved here',
  moved_out: 'moved away',
}

export function ChangeList({ diff, order, reviewed, current, callerFirst, onFlip, onPick, onReview }: Props) {
  const done = order.filter((position) => reviewed.has(reviewKey(diff.changes[position]))).length
  const pathOf = (position: number) => diff.changes[position].new_path ?? diff.changes[position].old_path ?? ''
  return (
    <aside className="mchanges" aria-label="Changes">
      <header>
        <strong>
          {order.length} {order.length === 1 ? 'change' : 'changes'}
        </strong>
        <span className="mchanges-progress">{done} reviewed</span>
      </header>
      <button
        type="button"
        className="mchanges-flip"
        onClick={onFlip}
        title="Read what is used before what uses it, or the other way round"
      >
        Order: {callerFirst ? 'users first' : 'used first'}
      </button>
      {order.length === 0 && <p className="minspect-empty">Nothing to review at this level.</p>}
      <ol>
        {order.map((position, at) => {
          const change = diff.changes[position]
          // A file is named once, above its first change.
          const heading = at === 0 || pathOf(order[at - 1]) !== pathOf(position) ? pathOf(position) : null
          // Tests are read after the code, and are not on the map.
          const isTest = diff.tests.has(position)
          const firstTest = isTest && (at === 0 || !diff.tests.has(order[at - 1]))
          const isDone = reviewed.has(reviewKey(change))
          // Two fields called `name` are told apart by the type each is in.
          const outer = change.parent === null ? null : diff.changes[change.parent]
          const holder = outer && !['file', 'document', 'directory'].includes(outer.kind) ? outer.label : null
          // What holds other changes is listed for its own lines: theirs are on their own rows.
          const lines = change.children.length > 0 && change.state === 'modified' ? change.own : change.total
          return (
            <li key={position}>
              {firstTest && <div className="mchanges-group">Tests · not on the map</div>}
              {heading !== null && <div className="mchanges-file">{heading}</div>}
              <div
                className={`mchanges-row${position === current ? ' is-current' : ''}${isDone ? ' is-done' : ''}`}
                ref={(element) => {
                  if (position === current) element?.scrollIntoView({ block: 'nearest' })
                }}
              >
                <button type="button" className="mchanges-pick" onClick={() => onPick(position)} title={change.new_id ?? change.old_id ?? ''}>
                  <span className={`mchanges-dot t-${toneOf(change)}`} aria-hidden="true" />
                  <span className="mchanges-name">
                    {!change.label.startsWith(`${change.lang_kind} `) && (
                      <span className="mword">{change.lang_kind} </span>
                    )}
                    {change.label}
                  </span>
                  <span className="mchanges-what">
                    {holder && <span>in {holder} · </span>}
                    {WHAT[change.state]}
                    {change.state === 'modified' && change.kinds.length > 0 && ` (${change.kinds.join(', ')})`}
                    {lines.added > 0 && <span className="mchanges-plus"> +{lines.added}</span>}
                    {lines.removed > 0 && <span className="mchanges-minus"> −{lines.removed}</span>}
                  </span>
                </button>
                <input
                  type="checkbox"
                  checked={isDone}
                  aria-label={`Reviewed: ${change.label}`}
                  title="Reviewed (R)"
                  onChange={(event) => onReview(position, event.target.checked)}
                />
              </div>
            </li>
          )
        })}
      </ol>
    </aside>
  )
}
