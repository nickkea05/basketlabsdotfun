import { useEffect, useRef, useState } from 'react'

// Open/close state for a popover anchored to a trigger. Closes on outside
// pointer-down and Escape. Attach `ref` to the wrapper that contains both the
// trigger and the popover.
export function usePopover() {
  const [open, setOpen] = useState(false)
  const ref = useRef(null)

  useEffect(() => {
    if (!open) return
    const onDown = (e) => {
      if (ref.current && !ref.current.contains(e.target)) setOpen(false)
    }
    const onKey = (e) => e.key === 'Escape' && setOpen(false)
    document.addEventListener('mousedown', onDown)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  return { open, setOpen, toggle: () => setOpen((o) => !o), close: () => setOpen(false), ref }
}
