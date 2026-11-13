import { useEffect, useRef } from 'react'
import type { Change, ChangeSet, ChangeState } from '../api'
import { progressOf, reviewKey, type TreeRow } from '../model'

const STATE_LABEL: Record<ChangeState, string> = {
  added: 'added',
  removed: 'removed',
  modified: 'modified',
  renamed: 'renamed',
  moved_in: 'moved in',
  moved_out: 'moved out',
}

interface Props {
  changeSet: ChangeSet
  rows: TreeRow[]
  selected: number | null
  reviewed: Set<string>
  onSelect: (index: number) => void
  onToggleCollapsed: (index: number) => void
  onToggleReviewed: (index: number) => void
}

export function ChangeTree({
  changeSet,
  rows,
  selected,
  reviewed,
  onSelect,
  onToggleCollapsed,
  onToggleReviewed,
}: Props) {
  const selectedRow = useRef<HTMLLIElement | null>(null)

  // Keep the selected row in view when the keyboard moves it.
  useEffect(() => {
    selectedRow.current?.scrollIntoView({ block: 'nearest' })
  }, [selected])

  if (rows.length === 0) {
    return <p className="tree-empty">Nothing matches the filters.</p>
  }

  return (
    <ul className="tree" role="tree" aria-label="Changed scopes">
      {rows.map((row) => {
        const change = changeSet.changes[row.index]
        const isSelected = row.index === selected
        const progress = progressOf(changeSet.changes, row.index, reviewed)
        const done = progress.total > 0 && progress.reviewed === progress.total
        return (
          <li
            key={row.index}
            ref={isSelected ? selectedRow : undefined}
            role="treeitem"
            aria-selected={isSelected}
            aria-expanded={row.hasChildren ? !row.collapsed : undefined}
            aria-level={row.depth + 1}
            className={`tree-row${isSelected ? ' is-selected' : ''}${done ? ' is-done' : ''}`}
            style={{ paddingInlineStart: `${row.depth * 14 + 6}px` }}
            onClick={() => onSelect(row.index)}
          >
            <button
              type="button"
              className="twisty"
              tabIndex={-1}
              aria-label={row.collapsed ? 'Expand' : 'Collapse'}
              style={{ visibility: row.hasChildren ? 'visible' : 'hidden' }}
              onClick={(event) => {
                event.stopPropagation()
                onToggleCollapsed(row.index)
              }}
            >
              {row.collapsed ? '▸' : '▾'}
            </button>

            <span className={`tree-label kind-${change.kind}`} title={change.new_id ?? change.old_id ?? ''}>
              {change.old_label !== null && (
                <>
                  <span className="old-label">{change.old_label}</span>
                  <span className="arrow"> → </span>
                </>
              )}
              {row.label}
              {change.kind === 'directory' && '/'}
            </span>

            <RowDetails change={change} />

            <span className="tree-counts">
              {change.total.added > 0 && <span className="count-added">+{change.total.added}</span>}
              {change.total.removed > 0 && (
                <span className="count-removed">-{change.total.removed}</span>
              )}
            </span>

            {row.reviewable ? (
              <input
                type="checkbox"
                className="reviewed-box"
                tabIndex={-1}
                aria-label="Reviewed"
                title="Reviewed (r)"
                checked={reviewed.has(reviewKey(change))}
                onClick={(event) => event.stopPropagation()}
                onChange={() => onToggleReviewed(row.index)}
              />
            ) : (
              <span className="tree-progress" title="Reviewed changes inside">
                {progress.total > 0 ? `${progress.reviewed}/${progress.total}` : ''}
              </span>
            )}
          </li>
        )
      })}
    </ul>
  )
}

function RowDetails({ change }: { change: Change }) {
  if (change.kind === 'directory') return <span className="tree-details" />
  const containerOnly =
    change.state === 'modified' &&
    change.own.added + change.own.removed === 0 &&
    change.children.length > 0
  return (
    <span className="tree-details">
      {!containerOnly && (
        <span className={`state state-${change.state}`}>{STATE_LABEL[change.state]}</span>
      )}
      {change.kinds.length > 0 && <span className="kinds">{change.kinds.join(', ')}</span>}
      {change.moved_delta !== null && (
        <span className="kinds">
          {change.moved_delta.added + change.moved_delta.removed === 0
            ? 'text unchanged'
            : `text +${change.moved_delta.added} -${change.moved_delta.removed}`}
        </span>
      )}
    </span>
  )
}
