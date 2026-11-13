import type { ChangeSet, Meta, Target } from '../api'
import type { Filters, Progress } from '../model'
import { TargetPicker } from './TargetPicker'

interface Props {
  meta: Meta | null
  target: Target
  onTarget: (target: Target) => void
  filters: Filters
  onFilters: (filters: Filters) => void
  changeSet: ChangeSet | null
  progress: Progress
  loading: boolean
  onReload: () => void
}

export function Toolbar({
  meta,
  target,
  onTarget,
  filters,
  onFilters,
  changeSet,
  progress,
  loading,
  onReload,
}: Props) {
  return (
    <header className="toolbar">
      <div className="toolbar-group">
        <a href="#map/changes" title="These changes, drawn on a map of the repository">Map</a>
      </div>
      <div className="toolbar-group">
        <strong className="repo-name">{meta?.name ?? 'mapdiff'}</strong>
        {meta?.branch && <span className="branch">{meta.branch}</span>}
      </div>

      <div className="toolbar-group">
        <TargetPicker meta={meta} target={target} onTarget={onTarget} />

        <button type="button" onClick={onReload} disabled={loading} title="Read the changes again">
          {loading ? 'Loading' : 'Reload'}
        </button>
      </div>

      <div className="toolbar-group">
        <label>
          <span className="field-label">Kind</span>
          <select
            value={filters.kind}
            onChange={(event) =>
              onFilters({ ...filters, kind: event.target.value as Filters['kind'] })
            }
          >
            <option value="all">All changes</option>
            <option value="signatures">Signatures</option>
            <option value="bodies">Bodies</option>
          </select>
        </label>
        <label>
          <span className="field-label">Access</span>
          <select
            value={filters.access}
            onChange={(event) =>
              onFilters({ ...filters, access: event.target.value as Filters['access'] })
            }
          >
            <option value="all">Any</option>
            <option value="public">Public only</option>
          </select>
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={filters.hideFormatting}
            onChange={(event) => onFilters({ ...filters, hideFormatting: event.target.checked })}
          />
          Hide formatting-only
        </label>
      </div>

      <div className="toolbar-group toolbar-summary">
        {changeSet && (
          <>
            <span>
              {changeSet.stats.scopes} {changeSet.stats.scopes === 1 ? 'scope' : 'scopes'} in{' '}
              {changeSet.stats.files} {changeSet.stats.files === 1 ? 'file' : 'files'}
            </span>
            <span className="count-added">+{changeSet.stats.added}</span>
            <span className="count-removed">-{changeSet.stats.removed}</span>
            <span className="progress" title="Changes marked as reviewed">
              {progress.reviewed} of {progress.total} reviewed
            </span>
          </>
        )}
      </div>
    </header>
  )
}
