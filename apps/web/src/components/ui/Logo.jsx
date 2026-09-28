import { useState } from 'react'

/** Token logo with a lettered fallback when the icon is missing or fails to load. */
export default function Logo({ token, size = 40, className = '' }) {
  const [broken, setBroken] = useState(false)
  const letter = (token.symbol || token.name || '?')[0]?.toUpperCase()
  const style = { width: size, height: size, fontSize: Math.max(10, size * 0.4) }
  if (!token.icon || broken) {
    return (
      <div className={`logo logo-fallback ${className}`} style={style} aria-hidden="true">
        {letter}
      </div>
    )
  }
  return <img className={`logo ${className}`} style={style} src={token.icon} alt="" loading="lazy" onError={() => setBroken(true)} />
}
