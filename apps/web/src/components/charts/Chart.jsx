export default function Chart({ data, width = 800, height = 300 }) {
  const min = Math.min(...data)
  const max = Math.max(...data)
  const pad = 12
  const x = (i) => (i / (data.length - 1)) * width
  const y = (v) => pad + (1 - (v - min) / (max - min || 1)) * (height - pad * 2)

  const line = data.map((v, i) => `${i ? 'L' : 'M'}${x(i).toFixed(1)},${y(v).toFixed(1)}`).join(' ')
  const area = `${line} L${width},${height} L0,${height} Z`
  const last = data[data.length - 1]

  return (
    <svg className="chart" viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none">
      <defs>
        <linearGradient id="fill" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#ffffff" stopOpacity="0.14" />
          <stop offset="100%" stopColor="#ffffff" stopOpacity="0" />
        </linearGradient>
      </defs>
      {[0.25, 0.5, 0.75].map((f) => (
        <line
          key={f}
          x1="0"
          x2={width}
          y1={pad + f * (height - pad * 2)}
          y2={pad + f * (height - pad * 2)}
          stroke="rgba(255,255,255,0.05)"
          strokeDasharray="3 6"
        />
      ))}
      <path d={area} fill="url(#fill)" />
      <path d={line} fill="none" stroke="#f1f1f3" strokeWidth="1.5" vectorEffect="non-scaling-stroke" />
      <line x1="0" x2={width} y1={y(last)} y2={y(last)} stroke="rgba(255,63,128,0.45)" strokeDasharray="2 5" />
      <circle cx={width} cy={y(last)} r="3.5" fill="#ff3f80" />
    </svg>
  )
}
