import { useMemo, useState } from 'react'
import type { Change, ChangeDetail, Location, Row } from '../api'
import { foldRows, pairRows, type DisplayRow } from '../model'

interface Props {
  change: Change
  detail: ChangeDetail | null
  error: string | null
  /** Jumps to the other end of a move. */
  onOpenCounterpart: (id: string) => void
}

const STATE_SENTENCE: Record<Change['state'], string> = {
  added: 'Added',
  removed: 'Removed',
  modified: 'Modified',
  renamed: 'Renamed',
  moved_in: 'Moved here',
  moved_out: 'Moved away',
}

export function DiffView({ change, detail, error, onOpenCounterpart }: Props) {
  return (
    <section className="detail" aria-label="Before and after">
      <header className="detail-header">
        <h2 className="detail-id">{change.new_id ?? change.old_id}</h2>
        <p className="detail-facts">
          <span className={`state state-${change.state}`}>{STATE_SENTENCE[change.state]}</span>
          <span>{change.lang_kind}</span>
          {change.kind !== 'directory' && change.kind !== 'file' && change.kind !== 'document' && (
            <span>{change.access}</span>
          )}
          {change.kinds.length > 0 && <span>changed: {change.kinds.join(', ')}</span>}
          {change.old_label !== null && <span>was {change.old_label}</span>}
          {change.counterpart !== null && (
            <button
              type="button"
              className="link"
              onClick={() => onOpenCounterpart(change.counterpart as string)}
            >
              {change.state === 'moved_in' ? 'from' : 'to'} {change.counterpart}
            </button>
          )}
        </p>
      </header>

      {error !== null && <p className="detail-note is-error">{error}</p>}
      {error === null && detail === null && <p className="detail-note">Loading…</p>}
      {error === null && detail !== null && detail.note !== null && (
        <p className="detail-note">{detail.note}</p>
      )}
      {error === null && detail !== null && detail.note === null && (
        // Keyed by scope so the unfolded runs of one scope never carry over.
        <DiffTable key={`${detail.index}:${detail.id}`} detail={detail} />
      )}
    </section>
  )
}

function DiffTable({ detail }: { detail: ChangeDetail }) {
  const [open, setOpen] = useState<Set<number>>(() => new Set())
  const paired = useMemo(() => pairRows(detail.rows), [detail.rows])
  const rows = useMemo(() => foldRows(paired, open), [paired, open])

  // One side only: a scope that was added or removed whole.
  const oneSided = detail.old === null || detail.new === null

  return (
    <div className="diff-scroll">
      <table className={`diff${oneSided ? ' is-one-sided' : ''}`}>
        <thead>
          <tr>
            {detail.old !== null && (
              <th colSpan={2} scope="col">
                Before <LocationLabel location={detail.old} />
              </th>
            )}
            {detail.new !== null && (
              <th colSpan={2} scope="col">
                After <LocationLabel location={detail.new} />
              </th>
            )}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, position) =>
            row.kind === 'fold' ? (
              <tr key={`fold-${row.key}`} className="fold">
                <td colSpan={oneSided ? 2 : 4}>
                  <button
                    type="button"
                    onClick={() => setOpen((current) => new Set(current).add(row.key))}
                  >
                    Show {row.hidden.length} unchanged lines
                  </button>
                </td>
              </tr>
            ) : (
              <DiffRow
                key={position}
                row={row}
                showOld={detail.old !== null}
                showNew={detail.new !== null}
              />
            ),
          )}
        </tbody>
      </table>
    </div>
  )
}

function LocationLabel({ location }: { location: Location }) {
  return (
    <span className="location">
      {location.path}:{location.first_line}
    </span>
  )
}

function DiffRow({
  row,
  showOld,
  showNew,
}: {
  row: DisplayRow
  showOld: boolean
  showNew: boolean
}) {
  return (
    <tr>
      {showOld && <Cell row={row.left} side="old" />}
      {showNew && <Cell row={row.right} side="new" />}
    </tr>
  )
}

function Cell({ row, side }: { row: Row | null; side: 'old' | 'new' }) {
  if (row === null) {
    return (
      <>
        <td className="line-number is-empty" />
        <td className="line-text is-empty" />
      </>
    )
  }
  const tone = row.tag === 'equal' ? '' : row.tag === 'delete' ? ' is-deleted' : ' is-inserted'
  const number = side === 'old' ? row.old_line : row.new_line
  return (
    <>
      <td className={`line-number${tone}`}>{number}</td>
      <td className={`line-text${tone}`}>
        {row.spans.map((span, position) => {
          const classes = [
            span.class !== null ? `tok-${span.class}` : '',
            span.changed ? 'is-changed' : '',
          ]
            .filter(Boolean)
            .join(' ')
          return classes === '' ? (
            span.text
          ) : (
            <span key={position} className={classes}>
              {span.text}
            </span>
          )
        })}
      </td>
    </>
  )
}
