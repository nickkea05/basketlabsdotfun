import { useMemo, useRef } from 'react'
import { tokenize } from './highlight.js'
import { useCopy } from '../../features/wallet/useCopy.js'
import { Icon } from '../ui/index.js'
import './code.css'

/**
 * A code surface that behaves like an editor without pretending to be an IDE:
 * gutter with line numbers, syntax colouring, tab/enter indentation, the
 * current line highlighted. Pass `onChange` for editable, omit for read-only.
 * The highlighted <pre> and the transparent <textarea> share one grid cell so
 * they never drift.
 */
export default function CodeEditor({ value, onChange, file, badge = 'JS', minLines = 8, actions, status, placeholder }) {
  const ta = useRef(null)
  const readOnly = !onChange
  const [copied, copy] = useCopy(value)

  const lines = useMemo(() => Math.max(minLines, value.split('\n').length), [value, minLines])
  const tokens = useMemo(() => tokenize(value), [value])

  const onKeyDown = (e) => {
    const el = e.currentTarget
    const { selectionStart: s, selectionEnd: end } = el
    if (e.key === 'Tab') {
      e.preventDefault()
      if (e.shiftKey) {
        // outdent current line by up to two spaces
        const ls = value.lastIndexOf('\n', s - 1) + 1
        const n = /^ {1,2}/.exec(value.slice(ls))?.[0].length ?? 0
        if (n) {
          onChange(value.slice(0, ls) + value.slice(ls + n))
          queueMicrotask(() => el.setSelectionRange(s - n, end - n))
        }
        return
      }
      onChange(value.slice(0, s) + '  ' + value.slice(end))
      queueMicrotask(() => el.setSelectionRange(s + 2, s + 2))
    } else if (e.key === 'Enter') {
      e.preventDefault()
      const ls = value.lastIndexOf('\n', s - 1) + 1
      const indent = /^[ \t]*/.exec(value.slice(ls, s))[0]
      const opens = /[{([]\s*$/.test(value.slice(ls, s))
      const closesNext = /^\s*[})\]]/.test(value.slice(end))
      const extra = opens ? '  ' : ''
      let insert = '\n' + indent + extra
      let caret = s + insert.length
      if (opens && closesNext) insert += '\n' + indent
      onChange(value.slice(0, s) + insert + value.slice(end))
      queueMicrotask(() => el.setSelectionRange(caret, caret))
    }
  }

  return (
    <div className={`code ${readOnly ? 'readonly' : ''}`}>
      <div className="code-head">
        <div className="code-tab">
          <span className="code-dot" />
          {file}
        </div>
        <div className="code-head-right">
          {status}
          <span className="code-badge">{badge}</span>
          <button className="code-act" onClick={copy} title="Copy">
            {copied ? <Icon.Check /> : <Icon.Copy />}
          </button>
          {actions}
        </div>
      </div>
      <div className="code-body">
        <div className="code-gutter" aria-hidden="true">
          {Array.from({ length: lines }, (_, i) => (
            <span key={i}>{i + 1}</span>
          ))}
        </div>
        <div className="code-cell">
          <pre className="code-hl" aria-hidden={!readOnly}>
            {tokens.map((t, i) => (
              <span key={i} className={t.cls}>
                {t.text}
              </span>
            ))}
            {'\n'}
          </pre>
          {!readOnly && (
            <textarea
              ref={ta}
              className="code-ta"
              value={value}
              onChange={(e) => onChange(e.target.value)}
              onKeyDown={onKeyDown}
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              rows={lines}
              placeholder={placeholder}
              aria-label={file}
            />
          )}
        </div>
      </div>
    </div>
  )
}
