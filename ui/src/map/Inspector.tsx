// The panel beside the map: what the selected box is, who uses it, and
// what it uses.

import type { Change, NodeInfo, Use } from '../api'
import type { Cause } from '../layout/types.ts'

interface Props {
  info: NodeInfo
  /** The change being read, when it is drawn in this box. */
  change: { change: Change; reviewed: boolean; onReview(done: boolean): void } | null
  /** Changes elsewhere that this box can see: it uses what they changed. */
  causes: Cause[]
  /** Which list to bring into view: set by the hover card's buttons. */
  focus: 'callers' | 'dependencies' | null
  onJump(use: Use): void
  onTrace(): void
  onShow(): void
  onClose(): void
}

function UseList({ title, empty, uses, onJump, scrollTo }: {
  title: string
  empty: string
  uses: Use[]
  onJump(use: Use): void
  scrollTo: boolean
}) {
  return (
    <section
      className="minspect-section"
      ref={(element) => {
        if (scrollTo) element?.scrollIntoView({ block: 'start' })
      }}
    >
      <h3>
        {title} <span className="minspect-count">{uses.length}</span>
      </h3>
      {uses.length === 0 && <p className="minspect-empty">{empty}</p>}
      <ul>
        {uses.map((use) => (
          <li key={use.id}>
            <button type="button" className="minspect-use" onClick={() => onJump(use)} title={use.id}>
              <span className="minspect-use-name">
                <span className="mword">{use.lang_kind} </span>
                {use.label}
                {use.count > 1 && <span className="minspect-times"> ×{use.count}</span>}
              </span>
              <span className="minspect-use-path">{use.path}</span>
              {use.places[0] && (
                <code className="minspect-use-line">
                  {use.places[0].line}: {use.places[0].text}
                </code>
              )}
            </button>
          </li>
        ))}
      </ul>
    </section>
  )
}

const SENTENCE: Record<Change['state'], string> = {
  added: 'Added',
  removed: 'Removed',
  modified: 'Modified',
  renamed: 'Renamed',
  moved_in: 'Moved here',
  moved_out: 'Moved away',
}

export function Inspector({ info, change, causes, focus, onJump, onTrace, onShow, onClose }: Props) {
  const where =
    info.first_line > 0 ? `${info.path}:${info.first_line}-${info.last_line}` : info.path
  return (
    <aside className="minspect" aria-label="Selected scope">
      <header>
        <div className="minspect-title">
          <span className="mword">{info.lang_kind}</span> <strong>{info.label}</strong>
        </div>
        <button type="button" onClick={onClose} aria-label="Close" title="Clear the selection (Esc)">
          ×
        </button>
      </header>
      <div className="minspect-where">{where}</div>
      {info.side === 'base' && <p className="minspect-note">Only there before the change.</p>}
      {change && (
        <section className="minspect-change">
          <div>
            <strong>{SENTENCE[change.change.state]}</strong>
            {change.change.old_label && change.change.state === 'renamed' && ` from ${change.change.old_label}`}
            {change.change.kinds.length > 0 && `: ${change.change.kinds.join(', ')}`}
            <span className="mchanges-plus"> +{change.change.total.added}</span>
            <span className="mchanges-minus"> −{change.change.total.removed}</span>
          </div>
          {change.change.counterpart && (
            <div className="minspect-where">
              {change.change.state === 'moved_in' ? 'from' : 'to'} {change.change.counterpart}
            </div>
          )}
          <label>
            <input type="checkbox" checked={change.reviewed} onChange={(event) => change.onReview(event.target.checked)} />{' '}
            Reviewed
          </label>
        </section>
      )}
      {causes.length > 0 && (
        <section className="minspect-affected">
          <strong>Not changed, but it uses:</strong>
          <ul>
            {causes.map((cause) => (
              <li key={`${cause.at}\n${cause.label}`}>
                <code>{cause.label}</code> ({cause.reasons.join(', ')})
              </li>
            ))}
          </ul>
        </section>
      )}
      {info.signature && <pre className="minspect-signature">{info.signature}</pre>}
      <div className="minspect-actions">
        <button type="button" onClick={onShow}>Show on map</button>
        <button type="button" onClick={onTrace} title="Its code, with who uses it and what it uses (T)">
          Trace
        </button>
      </div>
      <UseList
        title="Used by"
        empty="Nothing in this repository uses it."
        uses={info.callers}
        onJump={onJump}
        scrollTo={focus === 'callers'}
      />
      <UseList
        title="Uses"
        empty="It uses nothing in this repository."
        uses={info.dependencies}
        onJump={onJump}
        scrollTo={focus === 'dependencies'}
      />
    </aside>
  )
}
