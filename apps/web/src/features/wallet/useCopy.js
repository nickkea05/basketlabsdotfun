import { useCallback, useEffect, useRef, useState } from 'react'

/** Copy `text` to the clipboard; `copied` flips true for a moment afterwards. */
export function useCopy(text, resetMs = 1200) {
  const [copied, setCopied] = useState(false)
  const timer = useRef(null)
  useEffect(() => () => clearTimeout(timer.current), [])
  const copy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      clearTimeout(timer.current)
      timer.current = setTimeout(() => setCopied(false), resetMs)
    } catch {}
  }, [text, resetMs])
  return [copied, copy]
}
