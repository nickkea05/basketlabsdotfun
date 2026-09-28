import { useEffect, useRef } from 'react'

// A single pink light that travels the panel's outline. Cruises along the long
// edges, swells in speed through the top-right and bottom-left corners, and
// settles back to cruise two-thirds of the way down the following side.
//
// Drawn on a canvas so the falloff is continuous: a smooth intensity curve
// along the path drives colour temperature (deep pink -> white-hot core),
// stroke width and alpha at every sample, with no discrete layers.
//
// Every instance shares one wall clock and one lap `period`, so lights on
// different panels stay in phase. Cruise speed is identical everywhere; a
// larger panel makes up its longer perimeter with a bigger corner boost, so
// two lights passing each other on their slow stretches move at the same pace.
// At t = 0 (mod period) the head sits on `edge` at `cross` of the width, which
// is how two stacked panels are made to meet at the seam.

export default function Alive({ radius = 18, cruise = 200, length = 620, period = 8, edge = 'bottom', cross = 0.42, direction = 1 }) {
  const ref = useRef(null)

  useEffect(() => {
    const canvas = ref.current
    const host = canvas.parentElement
    const ctx = canvas.getContext('2d')
    // Bloom is drawn at quarter resolution and blurred once per frame.
    const bloom = document.createElement('canvas')
    const bctx = bloom.getContext('2d')
    const BS = 0.25
    let raf = 0
    let geom = null

    const build = () => {
      const W = host.offsetWidth
      const H = host.offsetHeight
      const dpr = Math.min(window.devicePixelRatio || 1, 2)
      canvas.width = Math.round((W + 40) * dpr)
      canvas.height = Math.round((H + 40) * dpr)
      canvas.style.width = `${W + 40}px`
      canvas.style.height = `${H + 40}px`
      ctx.setTransform(dpr, 0, 0, dpr, 20 * dpr, 20 * dpr) // 20px bleed for glow
      bloom.width = Math.ceil((W + 40) * BS)
      bloom.height = Math.ceil((H + 40) * BS)
      bctx.setTransform(BS, 0, 0, BS, 20 * BS, 20 * BS)

      const r = radius
      const tw = W - 2 * r
      const th = H - 2 * r
      const c = (Math.PI * r) / 2
      const L = 2 * tw + 2 * th + 4 * c

      // Clockwise from the start of the top edge.
      const pointAt = (s) => {
        s = ((s % L) + L) % L
        if (s < tw) return [r + s, 0]
        s -= tw
        if (s < c) {
          const a = -Math.PI / 2 + (s / c) * (Math.PI / 2)
          return [W - r + r * Math.cos(a), r + r * Math.sin(a)]
        }
        s -= c
        if (s < th) return [W, r + s]
        s -= th
        if (s < c) {
          const a = (s / c) * (Math.PI / 2)
          return [W - r + r * Math.cos(a), H - r + r * Math.sin(a)]
        }
        s -= c
        if (s < tw) return [W - r - s, H]
        s -= tw
        if (s < c) {
          const a = Math.PI / 2 + (s / c) * (Math.PI / 2)
          return [r + r * Math.cos(a), H - r + r * Math.sin(a)]
        }
        s -= c
        if (s < th) return [0, H - r - s]
        s -= th
        const a = Math.PI + (s / c) * (Math.PI / 2)
        return [r + r * Math.cos(a), r + r * Math.sin(a)]
      }

      // Velocity profile: cruise plus a raised-cosine bump through the
      // top-right and bottom-left corners. `m` is the bump weight in [0, 1].
      const corners = [tw + c / 2, tw + c + th + c + tw + c / 2]
      const before = tw / 3 + c / 2
      const after = c / 2 + (th * 2) / 3
      const bumpAt = (s) => {
        let m = 0
        for (const s0 of corners) {
          const d = s - s0
          if (d < 0 && -d < before) m = Math.max(m, 0.5 * (1 + Math.cos((Math.PI * d) / before)))
          else if (d >= 0 && d < after) m = Math.max(m, 0.5 * (1 + Math.cos((Math.PI * d) / after)))
        }
        return m
      }

      // Integrate one lap for a given cruise/boost and return the time table.
      const N = 600
      const ds = L / N
      const table = (v, b) => {
        const t = [0]
        for (let i = 0; i < N; i++) t.push(t[i] + ds / (v * (1 + (b - 1) * bumpAt((i + 0.5) * ds))))
        return t
      }

      // Solve for the boost that makes a lap take exactly `period` at the
      // shared cruise speed. If the panel is too small to need any boost,
      // fall back to a slower cruise so the period still holds.
      let v = cruise
      let b = 1
      if (L / cruise <= period) {
        v = L / period
      } else {
        let lo = 1,
          hi = 60
        for (let i = 0; i < 40; i++) {
          const mid = (lo + hi) / 2
          if (table(v, mid)[N] > period) lo = mid
          else hi = mid
        }
        b = (lo + hi) / 2
      }
      const times = table(v, b)
      const T = times[N]
      const scale = T / period // residual from the solve; normalise so T == period exactly

      const sAt = (t) => {
        t = (((t * scale) % T) + T) % T
        let lo = 0,
          hi = N
        while (hi - lo > 1) {
          const mid = (lo + hi) >> 1
          if (times[mid] <= t) lo = mid
          else hi = mid
        }
        const f = (t - times[lo]) / (times[lo + 1] - times[lo])
        return (lo + f) * ds
      }
      const timeAt = (s) => {
        s = ((s % L) + L) % L
        const i = Math.min(N - 1, Math.floor(s / ds))
        return (times[i] + ((s - i * ds) / ds) * (times[i + 1] - times[i])) / scale
      }

      // Phase: where the head should be at t = 0 (mod period). Clockwise, the
      // top edge runs left->right and the bottom edge right->left.
      const x = cross * W
      let sTarget = edge === 'top' ? x - r : tw + 2 * c + th + (W - r - x)
      if (direction < 0) sTarget = L - sTarget
      const phase = timeAt(sTarget)

      // Direction flips the parametrisation; the speed profile mirrors with it.
      const P = (s) => pointAt(direction * s)

      geom = { W, H, P, sAt, phase }
    }

    // Intensity along the light. u = 0 at the head, negative behind.
    // Long soft tail behind, short nose ahead.
    const TAIL = 0.78 // fraction of `length` behind the head
    const profile = (u) => {
      if (u <= 0) {
        const x = -u / (length * TAIL)
        return x >= 1 ? 0 : Math.pow(Math.cos((Math.PI / 2) * x), 1.6)
      }
      const x = u / (length * (1 - TAIL))
      return x >= 1 ? 0 : Math.pow(Math.cos((Math.PI / 2) * x), 2.2)
    }

    const draw = (now) => {
      raf = requestAnimationFrame(draw)
      if (!geom) return
      const { W, H, P: pointAt, sAt, phase } = geom
      ctx.clearRect(-20, -20, W + 40, H + 40)
      // Shared wall clock (not a per-instance t0) so every panel stays in phase.
      const head = sAt(now / 1000 + phase)

      ctx.lineCap = 'round'
      ctx.lineJoin = 'round'
      const SAMPLES = 220
      const step = length / SAMPLES

      // Pass 1: bloom, at low res, then one blur on composite.
      bctx.clearRect(-20, -20, W + 40, H + 40)
      bctx.lineCap = 'round'
      bctx.globalCompositeOperation = 'lighter'
      let prev = pointAt(head - length * TAIL)
      for (let i = 1; i <= SAMPLES; i += 2) {
        const u = -length * TAIL + i * step
        const f = profile(u)
        const p = pointAt(head + u)
        if (f < 0.03) {
          prev = p
          continue
        }
        bctx.strokeStyle = `hsla(338, 100%, 60%, ${0.07 * f})`
        bctx.lineWidth = 6 + 14 * f
        bctx.beginPath()
        bctx.moveTo(prev[0], prev[1])
        bctx.lineTo(p[0], p[1])
        bctx.stroke()
        prev = p
      }
      ctx.save()
      ctx.setTransform(1, 0, 0, 1, 0, 0)
      ctx.globalCompositeOperation = 'lighter'
      ctx.filter = 'blur(6px)'
      ctx.drawImage(bloom, 0, 0, canvas.width, canvas.height)
      ctx.restore()

      // Pass 2: the filament. Colour temperature, width and alpha all derive
      // from f, so every sample is a smooth step from its neighbours.
      ctx.globalCompositeOperation = 'source-over'
      prev = pointAt(head - length * TAIL)
      for (let i = 1; i <= SAMPLES; i++) {
        const u = -length * TAIL + i * step
        const f = profile(u)
        const p = pointAt(head + u)
        if (f < 0.005) {
          prev = p
          continue
        }
        const light = 54 + 38 * Math.pow(f, 3.4) // deep pink tails -> pale core
        const sat = 100 - 25 * Math.pow(f, 6) // desaturate only at the very core
        ctx.strokeStyle = `hsla(338, ${sat}%, ${light}%, ${Math.min(1, 0.03 + 0.72 * Math.pow(f, 1.5))})`
        ctx.lineWidth = 0.6 + 1.3 * Math.pow(f, 1.7)
        ctx.beginPath()
        ctx.moveTo(prev[0], prev[1])
        ctx.lineTo(p[0], p[1])
        ctx.stroke()
        prev = p
      }
    }

    build()
    raf = requestAnimationFrame(draw)
    const ro = new ResizeObserver(build)
    ro.observe(host)
    return () => {
      ro.disconnect()
      cancelAnimationFrame(raf)
    }
  }, [radius, cruise, length, period, edge, cross, direction])

  return <canvas className="alive-canvas" ref={ref} aria-hidden="true" />
}
