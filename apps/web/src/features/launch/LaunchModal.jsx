import { useEffect, useState } from 'react'
import { BPS_TOTAL, BasketType, MIN_ASSETS, STEPS, buildLaunchParams, lintStrategy, submitLaunch, sumBps } from '@basketfun/core'
import { stepsFor, useDraft } from './useDraft.js'
import { Icon } from '../../components/ui/index.js'
import TypeStep from './steps/TypeStep.jsx'
import WalletStep from './steps/WalletStep.jsx'
import StrategyStep from './steps/StrategyStep.jsx'
import TestStep from './steps/TestStep.jsx'
import AssetsStep from './steps/AssetsStep.jsx'
import AllocationStep from './steps/AllocationStep.jsx'
import DetailsStep from './steps/DetailsStep.jsx'
import ReviewStep from './steps/ReviewStep.jsx'
import ResultStep from './steps/ResultStep.jsx'
import { useWallet } from '../wallet/useWallet.js'
import './launch.css'

const LABEL = {
  [STEPS.Type]: 'Type',
  [STEPS.Wallet]: 'Host wallet',
  [STEPS.Strategy]: 'Code',
  [STEPS.Test]: 'Test',
  [STEPS.Assets]: 'Assets',
  [STEPS.Allocation]: 'Weights',
  [STEPS.Details]: 'Details',
  [STEPS.Review]: 'Review',
}

// Can the user leave this step? Mirrors what the program will reject anyway.
function canContinue(d) {
  switch (d.step) {
    case STEPS.Type:
      return d.basketType != null
    case STEPS.Wallet:
      return d.hosts.length > 0 && d.assets.length > 0
    case STEPS.Strategy:
      return lintStrategy(d.strategy).length === 0
    case STEPS.Test:
      return !!d.strategyRun?.ok && d.assets.length > 0
    case STEPS.Assets:
      return d.assets.length >= MIN_ASSETS
    case STEPS.Allocation:
      return d.basketType === BasketType.Mirror || sumBps(d.weights) === BPS_TOTAL
    case STEPS.Details:
      return d.meta.name.trim().length > 0 && d.meta.symbol.trim().length > 0
    default:
      return true
  }
}

export default function LaunchModal({ open, onClose }) {
  const [draft, dispatch] = useDraft()
  const [attempted, setAttempted] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [confirm, setConfirm] = useState(false)
  const { address, openConnect } = useWallet()

  // Anything worth keeping? A finished dry run isn't; a chosen type is.
  const dirty =
    draft.step !== STEPS.Result &&
    (draft.basketType != null ||
      draft.assets.length > 0 ||
      draft.meta.name ||
      draft.hostWallet ||
      draft.hosts.length > 0 ||
      Object.values(draft.strategy).some((s) => s.trim()))

  const discard = () => {
    dispatch({ type: 'reset' })
    setAttempted(false)
    setConfirm(false)
    onClose()
  }
  const keep = () => {
    setConfirm(false)
    onClose()
  }
  const requestClose = () => {
    if (confirm) return
    if (dirty) setConfirm(true)
    else discard()
  }

  useEffect(() => {
    if (!open) return
    const onKey = (e) => {
      if (e.key !== 'Escape') return
      if (confirm) setConfirm(false)
      else requestClose()
    }
    document.addEventListener('keydown', onKey)
    const prev = document.body.style.overflow
    document.body.style.overflow = 'hidden'
    return () => {
      document.removeEventListener('keydown', onKey)
      document.body.style.overflow = prev
    }
  })

  if (!open) return null

  const steps = stepsFor(draft.basketType ?? BasketType.Fixed)
  const idx = steps.indexOf(draft.step)
  const isResult = draft.step === STEPS.Result
  const isReview = draft.step === STEPS.Review
  const ok = canContinue(draft)

  const next = async () => {
    if (!ok) {
      setAttempted(true)
      return
    }
    setAttempted(false)
    if (isReview) {
      // The creator signs and pays; no wallet, no transaction.
      if (!address) {
        openConnect()
        return
      }
      setSubmitting(true)
      const res = await submitLaunch(buildLaunchParams(draft, { creator: address }))
      setSubmitting(false)
      dispatch({ type: 'result', value: res })
      return
    }
    dispatch({ type: 'step', step: steps[idx + 1] })
  }
  const back = () => {
    setAttempted(false)
    if (idx > 0) dispatch({ type: 'step', step: steps[idx - 1] })
  }

  return (
    <div className="lm-overlay" onMouseDown={(e) => e.target === e.currentTarget && requestClose()}>
      <div className="lm" role="dialog" aria-modal="true" aria-label="Launch a basket">
        {confirm && (
          <div className="lm-confirm-veil" onMouseDown={(e) => e.target === e.currentTarget && setConfirm(false)}>
            <div className="lm-confirm raised" role="alertdialog" aria-label="Save progress?">
              <div className="lm-confirm-title">
                Keep <span className="muted">this draft?</span>
              </div>
              <p className="muted">You can pick up where you left off next time you press Launch, or start clean.</p>
              <div className="lm-confirm-actions">
                <button className="btn btn-ghost" onClick={discard}>
                  Discard
                </button>
                <button className="btn btn-white" autoFocus onClick={keep}>
                  Keep draft
                </button>
              </div>
            </div>
          </div>
        )}
        <header className="lm-head">
          <div className="lm-title">
            Launch <span className="muted">a basket</span>
          </div>
          {!isResult && (
            <ol className="rail">
              {steps.map((s, i) => (
                <li key={s} className={i < idx ? 'done' : i === idx ? 'now' : ''}>
                  <span className="rail-n mono">{i < idx ? <Icon.Check /> : i + 1}</span>
                  <span className="rail-l">{LABEL[s]}</span>
                </li>
              ))}
            </ol>
          )}
          <button className="icon-btn" onClick={requestClose} aria-label="Close">
            <Icon.Close />
          </button>
        </header>

        <div className="lm-body" key={draft.step}>
          {draft.step === STEPS.Type && <TypeStep draft={draft} dispatch={dispatch} />}
          {draft.step === STEPS.Wallet && <WalletStep draft={draft} dispatch={dispatch} />}
          {draft.step === STEPS.Strategy && <StrategyStep draft={draft} dispatch={dispatch} />}
          {draft.step === STEPS.Test && <TestStep draft={draft} dispatch={dispatch} />}
          {draft.step === STEPS.Assets && <AssetsStep draft={draft} dispatch={dispatch} />}
          {draft.step === STEPS.Allocation && <AllocationStep draft={draft} dispatch={dispatch} attempted={attempted} />}
          {draft.step === STEPS.Details && <DetailsStep draft={draft} dispatch={dispatch} />}
          {draft.step === STEPS.Review && <ReviewStep draft={draft} creator={address} />}
          {draft.step === STEPS.Result && <ResultStep draft={draft} />}
        </div>

        <footer className="lm-foot">
          {isResult ? (
            <>
              <button className="btn btn-ghost raised" onClick={() => dispatch({ type: 'reset' })}>
                Start over
              </button>
              <button className="btn btn-white" onClick={discard}>
                Done
              </button>
            </>
          ) : (
            <>
              <button className="btn btn-ghost raised" onClick={back} disabled={idx === 0}>
                <Icon.Back /> Back
              </button>
              <div className="lm-foot-right">
                {attempted && !ok && <span className="foot-err">{hint(draft)}</span>}
                <button className={`btn ${isReview ? 'btn-accent' : 'btn-white'} ${ok ? '' : 'soft-disabled'}`} onClick={next} disabled={submitting}>
                  {submitting ? 'Building transaction…' : isReview ? (address ? 'Launch' : 'Connect wallet to launch') : 'Continue'}
                </button>
              </div>
            </>
          )}
        </footer>
      </div>
    </div>
  )
}

function hint(d) {
  switch (d.step) {
    case STEPS.Type:
      return 'Pick a type'
    case STEPS.Wallet:
      return 'Add at least one wallet'
    case STEPS.Strategy:
      return 'All three sections must lint clean'
    case STEPS.Test:
      return 'Run the strategy and get a book first'
    case STEPS.Assets:
      return `Pick at least ${MIN_ASSETS} assets`
    case STEPS.Allocation:
      return 'Weights must total 100%'
    case STEPS.Details:
      return 'Name and ticker are required'
    default:
      return ''
  }
}
