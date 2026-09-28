import { useEffect, useState } from 'react'
import Nav from '../components/layout/Nav.jsx'
import Explore from '../pages/Explore.jsx'
import Token from '../pages/Token.jsx'
import Portfolio from '../pages/Portfolio.jsx'
import Leaderboard from '../pages/Leaderboard.jsx'
import LaunchModal from '../features/launch/LaunchModal.jsx'
import HowItWorks from '../features/how/HowItWorks.jsx'
import SearchModal from '../features/search/SearchModal.jsx'
import { WalletProvider } from '../features/wallet/WalletProvider.jsx'

export default function App() {
  return (
    <WalletProvider>
      <Shell />
    </WalletProvider>
  )
}

function Shell() {
  const [route, setRoute] = useState('explore')
  const [token, setToken] = useState(null)
  const [launching, setLaunching] = useState(false)
  const [how, setHow] = useState(false)
  // Advanced search: null when closed, otherwise the query it opened with plus a
  // counter so each open remounts the modal with fresh filters.
  const [search, setSearch] = useState(null)
  const openSearch = (q = '') => setSearch((s) => ({ q, n: (s?.n ?? 0) + 1 }))

  // ⌘K / Ctrl+K opens advanced search from anywhere.
  useEffect(() => {
    const onKey = (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        openSearch()
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [])

  const go = (r, arg) => {
    // "create", "how" and "search" are modals over whatever page you're on, not routes.
    if (r === 'create') {
      setLaunching(true)
      return
    }
    if (r === 'how') {
      setHow(true)
      return
    }
    if (r === 'search') {
      openSearch(typeof arg === 'string' ? arg : '')
      return
    }
    setToken(null)
    setRoute(r)
    window.scrollTo({ top: 0 })
  }
  const openToken = (t) => {
    setToken(t)
    setRoute('token')
    window.scrollTo({ top: 0 })
  }

  return (
    <div className="app">
      <Nav route={route} go={go} />
      {route === 'token' && token ? (
        <Token t={token} back={() => go('explore')} go={go} />
      ) : route === 'portfolio' ? (
        <Portfolio go={go} />
      ) : route === 'leaderboard' ? (
        <Leaderboard go={go} />
      ) : (
        <Explore openToken={openToken} go={go} />
      )}
      <LaunchModal open={launching} onClose={() => setLaunching(false)} />
      <HowItWorks open={how} onClose={() => setHow(false)} go={go} />
      <SearchModal key={search?.n ?? 0} open={!!search} initialQuery={search?.q ?? ''} onClose={() => setSearch(null)} onPick={openToken} />
    </div>
  )
}
