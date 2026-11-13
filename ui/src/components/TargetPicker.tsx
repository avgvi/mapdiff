// Choosing what to review: uncommitted or staged changes, a branch, one
// commit, a range, or a GitHub pull request.

import { useState } from 'react'
import { fetchPullRequest, type Meta, type Target } from '../api'

interface Props {
  meta: Meta | null
  target: Target
  onTarget: (target: Target) => void
}

export function TargetPicker({ meta, target, onTarget }: Props) {
  // The range box keeps its own text until it is applied.
  const [range, setRange] = useState(target.mode === 'range' ? target.spec : '')

  const defaultBranch = meta?.default_branch ?? null
  const onOtherBranch = defaultBranch !== null && meta?.branch !== defaultBranch
  const commits = meta?.commits ?? []

  const chooseMode = (mode: string) => {
    switch (mode) {
      case 'working':
      case 'staged':
        onTarget({ mode })
        break
      case 'branch':
        if (defaultBranch !== null) onTarget({ mode: 'branch', base: defaultBranch })
        break
      case 'commit':
        if (commits.length > 0) onTarget({ mode: 'commit', commit: commits[0].hash })
        break
      case 'range':
        onTarget({ mode: 'range', spec: range })
        break
    }
    setAsking(mode === 'pr')
  }
  // A pull request is fetched, then shown as the range it stands for.
  const [asking, setAsking] = useState(false)
  const [number, setNumber] = useState('')
  const [fetching, setFetching] = useState(false)
  const [problem, setProblem] = useState<string | null>(null)
  const openPullRequest = () => {
    const wanted = Number(number.replace('#', ''))
    if (!Number.isInteger(wanted) || wanted <= 0) return setProblem('Type the number of a pull request.')
    setFetching(true)
    setProblem(null)
    fetchPullRequest(wanted)
      .then((found) => {
        setAsking(false)
        setRange(found.spec)
        onTarget({ mode: 'range', spec: found.spec })
      })
      .catch((error: unknown) => setProblem(error instanceof Error ? error.message : String(error)))
      .finally(() => setFetching(false))
  }

  return (
    <>
        <label>
          <span className="field-label">Review</span>
          <select value={asking ? 'pr' : target.mode} onChange={(event) => chooseMode(event.target.value)}>
            <option value="working">Uncommitted changes</option>
            <option value="staged">Staged changes</option>
            {defaultBranch !== null && (
              <option value="branch">
                {onOtherBranch ? `This branch vs ${defaultBranch}` : 'A branch comparison'}
              </option>
            )}
            <option value="commit" disabled={commits.length === 0}>
              One commit
            </option>
            <option value="range">A range</option>
            <option value="pr">A pull request</option>
          </select>
        </label>

        {!asking && target.mode === 'branch' && (
          <label>
            <span className="field-label">Base</span>
            <select
              value={target.base}
              onChange={(event) => onTarget({ mode: 'branch', base: event.target.value })}
            >
              {(meta?.branches ?? [target.base]).map((branch) => (
                <option key={branch} value={branch}>
                  {branch}
                </option>
              ))}
            </select>
          </label>
        )}

        {!asking && target.mode === 'commit' && (
          <select
            className="commit-select"
            aria-label="Commit"
            value={target.commit}
            onChange={(event) => onTarget({ mode: 'commit', commit: event.target.value })}
          >
            {commits.map((commit) => (
              <option key={commit.hash} value={commit.hash}>
                {commit.short} {commit.subject}
              </option>
            ))}
          </select>
        )}

        {!asking && target.mode === 'range' && (
          <form
            className="range-form"
            onSubmit={(event) => {
              event.preventDefault()
              onTarget({ mode: 'range', spec: range.trim() })
            }}
          >
            <input
              aria-label="Range"
              placeholder="main..feature or v1.0...HEAD"
              value={range}
              onChange={(event) => setRange(event.target.value)}
              spellCheck={false}
            />
            <button type="submit">Show</button>
          </form>
        )}
      {asking && (
        <form
          className="range-form"
          onSubmit={(event) => {
            event.preventDefault()
            openPullRequest()
          }}
        >
          <input
            aria-label="Pull request number"
            placeholder="Pull request number"
            value={number}
            onChange={(event) => setNumber(event.target.value)}
            inputMode="numeric"
          />
          <button type="submit" disabled={fetching} title="Fetches it from origin, into refs/mapdiff/pr/">
            {fetching ? 'Fetching' : 'Open'}
          </button>
          {problem && <span className="picker-problem">{problem}</span>}
        </form>
      )}
    </>
  )
}
