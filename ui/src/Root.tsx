import { useEffect, useState } from 'react'
import App from './App.tsx'
import { MapView } from './map/MapView.tsx'

/** The reviewer, or at `#map` the map of the repository. */
export function Root() {
  const [hash, setHash] = useState(window.location.hash)
  useEffect(() => {
    const changed = () => setHash(window.location.hash)
    window.addEventListener('hashchange', changed)
    return () => window.removeEventListener('hashchange', changed)
  }, [])
  return hash.startsWith('#map') ? <MapView /> : <App />
}
