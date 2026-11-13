import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  ApiError,
  fetchChanges,
  fetchDetail,
  fetchMeta,
  fetchReviewed,
  saveReviewed,
  type Change,
  type ChangeDetail,
  type ChangeSet,
  type Meta,
  type Target,
} from './api'
import { ChangeTree } from './components/ChangeTree'
import { readTarget, writeTarget } from './target'
import { DiffView } from './components/DiffView'
import { Toolbar } from './components/Toolbar'
import {
  NO_FILTERS,
  changeId,
  initiallyCollapsed,
  keysToMark,
  progressOf,
  reviewKey,
  treeRows,
  type Filters,
} from './model'

/** Tells one entry from every other, across reloads of the same change set. */
function selectionKey(change: Change): string {
  return `${change.state}\u001f${changeId(change)}`
}

/** Changes only when the content of the change set does. */
function signatureOf(changeSet: ChangeSet): string {
  return changeSet.changes.map((change) => `${selectionKey(change)}\u001f${change.fingerprint}`).join('\n')
}

function isTyping(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLSelectElement ||
    target instanceof HTMLTextAreaElement
  )
}

export default function App() {
  const [meta, setMeta] = useState<Meta | null>(null)
  const [target, setTarget] = useState<Target>(readTarget)
  const [reloads, setReloads] = useState(0)
  const [changeSet, setChangeSet] = useState<ChangeSet | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [filters, setFilters] = useState<Filters>(NO_FILTERS)
  const [collapsed, setCollapsed] = useState<Set<number>>(() => new Set())
  const [selectedKey, setSelectedKey] = useState<string | null>(null)
  const [reviewed, setReviewed] = useState<Set<string>>(() => new Set())
  const [detail, setDetail] = useState<ChangeDetail | null>(null)
  const [detailError, setDetailError] = useState<string | null>(null)
  const signature = useRef('')

  useEffect(() => {
    fetchMeta().then(setMeta, (reason: unknown) => setError(messageOf(reason)))
    fetchReviewed().then((keys) => setReviewed(new Set(keys)), () => {})
  }, [])

  // Load the change set whenever the target changes or a reload is asked for.
  useEffect(() => {
    const controller = new AbortController()
    writeTarget(target)
    setLoading(true)
    fetchChanges(target, controller.signal).then(
      (loaded) => {
        setError(null)
        setLoading(false)
        const next = signatureOf(loaded)
        // A reload that found the same changes keeps what is open and selected.
        if (next !== signature.current) {
          signature.current = next
          setCollapsed(initiallyCollapsed(loaded))
          setDetail(null)
        }
        setChangeSet(loaded)
      },
      (reason: unknown) => {
        if (controller.signal.aborted) return
        setLoading(false)
        setChangeSet(null)
        signature.current = ''
        setError(messageOf(reason))
      },
    )
    return () => controller.abort()
  }, [target, reloads])

  // Uncommitted work changes while the page is in the background.
  useEffect(() => {
    if (target.mode !== 'working' && target.mode !== 'staged') return
    const reload = () => setReloads((count) => count + 1)
    window.addEventListener('focus', reload)
    return () => window.removeEventListener('focus', reload)
  }, [target.mode])

  const rows = useMemo(
    () => (changeSet === null ? [] : treeRows(changeSet, filters, collapsed)),
    [changeSet, filters, collapsed],
  )

  const selected = useMemo(() => {
    if (changeSet === null) return null
    const visible = new Set(rows.map((row) => row.index))
    // When the selected change is folded away or filtered out, the nearest
    // row above it that is still listed takes its place.
    let found: number | null = changeSet.changes.findIndex(
      (change) => selectionKey(change) === selectedKey,
    )
    while (found !== null && found > 0 && !visible.has(found)) {
      found = changeSet.changes[found].parent
    }
    if (found !== null && found > 0 && visible.has(found)) return found
    // Nothing selected yet, or it is gone: start at the first change.
    return rows.find((row) => row.reviewable)?.index ?? rows[0]?.index ?? null
  }, [changeSet, selectedKey, rows])

  const select = useCallback(
    (index: number) => {
      if (changeSet !== null) setSelectedKey(selectionKey(changeSet.changes[index]))
    },
    [changeSet],
  )

  // Load the before and after of the selected change.
  const selectedChange = changeSet !== null && selected !== null ? changeSet.changes[selected] : null
  const selectedFingerprint = selectedChange?.fingerprint ?? null
  useEffect(() => {
    if (changeSet === null || selected === null) return
    const controller = new AbortController()
    setDetail(null)
    setDetailError(null)
    fetchDetail(target, selected, changeId(changeSet.changes[selected]), controller.signal).then(
      setDetail,
      (reason: unknown) => {
        if (controller.signal.aborted) return
        // The files moved on since the list was made: read it again.
        if (reason instanceof ApiError && reason.status === 409) {
          setReloads((count) => count + 1)
        } else {
          setDetailError(messageOf(reason))
        }
      },
    )
    return () => controller.abort()
    // The fingerprint stands in for the change set: it changes exactly when
    // the selected scope's text does.
    // oxlint-disable-next-line react-hooks/exhaustive-deps
  }, [target, selected, selectedFingerprint])

  const toggleCollapsed = useCallback((index: number) => {
    setCollapsed((current) => {
      const next = new Set(current)
      if (!next.delete(index)) next.add(index)
      return next
    })
  }, [])

  const toggleReviewed = useCallback(
    (index: number) => {
      if (changeSet === null) return
      const keys = keysToMark(changeSet.changes, index)
      if (keys.length === 0) return
      const marking = !keys.every((key) => reviewed.has(key))
      // Show it at once; the server's answer then replaces the guess.
      const guess = new Set(reviewed)
      for (const key of keys) {
        if (marking) guess.add(key)
        else guess.delete(key)
      }
      setReviewed(guess)
      saveReviewed(keys, marking).then(
        (saved) => setReviewed(new Set(saved)),
        (reason: unknown) => {
          setReviewed(reviewed)
          setError(messageOf(reason))
        },
      )
      // After marking, move on to the next change still to review.
      if (marking) {
        const position = rows.findIndex((row) => row.index === index)
        const next = rows
          .slice(position + 1)
          .find((row) => row.reviewable && !guess.has(reviewKey(changeSet.changes[row.index])))
        if (next) select(next.index)
      }
    },
    [changeSet, reviewed, rows, select],
  )

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (isTyping(event.target) || event.metaKey || event.ctrlKey || event.altKey) return
      if (rows.length === 0) return
      const position = rows.findIndex((row) => row.index === selected)
      const row = position >= 0 ? rows[position] : null
      const go = (to: number | undefined) => {
        if (to !== undefined) select(to)
      }
      switch (event.key) {
        case 'j':
        case 'ArrowDown':
          go(rows[Math.min(position + 1, rows.length - 1)]?.index)
          break
        case 'k':
        case 'ArrowUp':
          go(rows[Math.max(position - 1, 0)]?.index)
          break
        case 'n':
          go(rows.slice(position + 1).find((candidate) => candidate.reviewable)?.index)
          break
        case 'p':
          go(
            rows
              .slice(0, Math.max(position, 0))
              .reverse()
              .find((candidate) => candidate.reviewable)?.index,
          )
          break
        case 'l':
        case 'ArrowRight':
          if (row?.hasChildren && row.collapsed) toggleCollapsed(row.index)
          else if (row?.hasChildren) go(rows[position + 1]?.index)
          break
        case 'h':
        case 'ArrowLeft':
          if (row?.hasChildren && !row.collapsed) toggleCollapsed(row.index)
          else if (row) {
            // Go up to the row this one sits under.
            go(
              rows
                .slice(0, position)
                .reverse()
                .find((candidate) => candidate.depth < row.depth)?.index,
            )
          }
          break
        case 'r':
        case ' ':
          if (row) toggleReviewed(row.index)
          break
        default:
          return
      }
      event.preventDefault()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [rows, selected, select, toggleCollapsed, toggleReviewed])

  const progress = useMemo(
    () => (changeSet === null || changeSet.changes.length === 0 ? { reviewed: 0, total: 0 } : progressOf(changeSet.changes, 0, reviewed)),
    [changeSet, reviewed],
  )

  const openCounterpart = useCallback(
    (id: string) => {
      if (changeSet === null || selectedChange === null) return
      const other = changeSet.changes.findIndex(
        (change) => changeId(change) === id && change.fingerprint === selectedChange.fingerprint,
      )
      if (other >= 0) select(other)
    },
    [changeSet, selectedChange, select],
  )

  // While the next scope loads, the previous one's text must not show under
  // the new heading.
  const shownDetail =
    detail !== null &&
    selectedChange !== null &&
    detail.index === selected &&
    detail.id === changeId(selectedChange)
      ? detail
      : null

  const isEmpty = changeSet !== null && changeSet.stats.files === 0

  return (
    <div className="app">
      <Toolbar
        meta={meta}
        target={target}
        onTarget={setTarget}
        filters={filters}
        onFilters={setFilters}
        changeSet={changeSet}
        progress={progress}
        loading={loading}
        onReload={() => setReloads((count) => count + 1)}
      />

      {error !== null && (
        <p className="banner" role="alert">
          {error}
        </p>
      )}

      <main className="panes">
        <nav className="pane-tree">
          {changeSet !== null && !isEmpty && (
            <ChangeTree
              changeSet={changeSet}
              rows={rows}
              selected={selected}
              reviewed={reviewed}
              onSelect={select}
              onToggleCollapsed={toggleCollapsed}
              onToggleReviewed={toggleReviewed}
            />
          )}
          {isEmpty && (
            <p className="tree-empty">
              No changes between {changeSet.base.label} and {changeSet.head.label}.
            </p>
          )}
          {changeSet === null && error === null && <p className="tree-empty">Loading…</p>}
        </nav>

        <div className="pane-detail">
          {selectedChange !== null ? (
            <DiffView
              change={selectedChange}
              detail={shownDetail}
              error={detailError}
              onOpenCounterpart={openCounterpart}
            />
          ) : (
            <p className="detail-note">
              {isEmpty ? 'Pick something else to review in the bar above.' : 'Pick a change on the left.'}
            </p>
          )}
        </div>
      </main>

      <footer className="hints">
        <kbd>j</kbd> <kbd>k</kbd> move
        <kbd>n</kbd> <kbd>p</kbd> next and previous change
        <kbd>←</kbd> <kbd>→</kbd> collapse and expand
        <kbd>r</kbd> mark reviewed
      </footer>
    </div>
  )
}

function messageOf(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason)
}
