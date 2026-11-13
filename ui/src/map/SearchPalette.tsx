// Find a scope by a few letters of its name or path, and jump to it.

import { useEffect, useRef, useState } from 'react'
import { fetchSearch, type Hit } from '../api'

export function SearchPalette({ onChoose, onClose }: { onChoose(hit: Hit): void; onClose(): void }) {
  const [text, setText] = useState('')
  const [found, setFound] = useState<{ for: string; hits: Hit[] }>({ for: '', hits: [] })
  const [picked, setPicked] = useState(0)
  const input = useRef<HTMLInputElement>(null)

  useEffect(() => input.current?.focus(), [])

  useEffect(() => {
    const wanted = text.trim()
    if (wanted === '') return
    let current = true
    // Wait for a pause in typing before asking.
    const timer = window.setTimeout(() => {
      fetchSearch(wanted)
        .then((hits) => {
          if (!current) return
          setFound({ for: wanted, hits })
          setPicked(0)
        })
        .catch(() => {})
    }, 90)
    return () => {
      current = false
      window.clearTimeout(timer)
    }
  }, [text])

  const hits = text.trim() === '' ? [] : found.hits
  const keyDown = (event: React.KeyboardEvent) => {
    if (event.key === 'Escape') onClose()
    else if (event.key === 'ArrowDown') setPicked((at) => Math.min(hits.length - 1, at + 1))
    else if (event.key === 'ArrowUp') setPicked((at) => Math.max(0, at - 1))
    else if (event.key === 'Enter' && hits[picked]) onChoose(hits[picked])
    else return
    event.preventDefault()
  }

  return (
    <div className="msearch-backdrop" onPointerDown={onClose}>
      <div className="msearch" role="dialog" aria-label="Find a scope" onPointerDown={(event) => event.stopPropagation()}>
        <input
          ref={input}
          type="text"
          value={text}
          placeholder="Find a function, type or file…"
          aria-label="Find a scope"
          onChange={(event) => setText(event.target.value)}
          onKeyDown={keyDown}
        />
        <ul role="listbox">
          {hits.map((hit, position) => (
            <li key={hit.id} role="option" aria-selected={position === picked}>
              <button
                type="button"
                className={position === picked ? 'is-picked' : ''}
                onPointerMove={() => setPicked(position)}
                onClick={() => onChoose(hit)}
              >
                <span>
                  <span className="mword">{hit.lang_kind} </span>
                  {hit.label}
                </span>
                <span className="msearch-path">{hit.id}</span>
              </button>
            </li>
          ))}
          {text.trim() !== '' && found.for === text.trim() && hits.length === 0 && (
            <li className="msearch-none">Nothing matches.</li>
          )}
        </ul>
      </div>
    </div>
  )
}
