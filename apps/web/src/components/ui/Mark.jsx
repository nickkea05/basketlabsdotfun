/**
 * The basketlabs.fun mark: an isometric stack of bars with two pink slats.
 * Inline so it inherits `color` for the white parts; pink stays the accent.
 * Source of truth for the artwork is `public/brand/basketlabs-mark.svg`.
 */
export default function Mark({ size = 22, className = '' }) {
  return (
    <svg className={`mark ${className}`} viewBox="8 6 48 56" width={(size * 48) / 56} height={size} fill="currentColor" aria-hidden="true">
      <polygon points="55.5,39.54 48.5,43.63 48.5,51.8 55.5,47.71" />
      <polygon points="47.5,44.2 40.5,48.29 40.5,56.46 47.5,52.37" />
      <polygon points="39.5,48.87 32.5,52.96 32.5,61.13 39.5,57.04" />
      <polygon points="8.5,39.54 31.5,52.96 31.5,61.13 8.5,47.71" />
      <polygon points="8.5,30.2 15.5,34.29 15.5,42.46 8.5,38.37" />
      <polygon points="16.5,34.87 23.5,38.96 23.5,47.13 16.5,43.04" />
      <polygon fill="var(--accent)" points="24.5,39.54 31.5,43.63 31.5,51.8 24.5,47.71" />
      <polygon fill="var(--accent)" points="55.5,30.21 32.5,43.63 32.5,51.8 55.5,38.38" />
      <polygon points="32,6.58 55.01,20 48,24.09 24.99,10.67" />
      <polygon points="55.5,20.87 48.5,24.96 48.5,33.13 55.5,29.04" />
      <polygon points="24,11.25 47.01,24.67 40,28.76 16.99,15.34" />
      <polygon points="47.5,25.54 40.5,29.63 40.5,37.8 47.5,33.71" />
      <polygon points="16,15.91 39.01,29.33 32,33.42 8.99,20" />
      <polygon points="39.5,30.2 32.5,34.29 32.5,42.46 39.5,38.37" />
      <polygon points="8.5,20.87 31.5,34.29 31.5,42.46 8.5,29.04" />
    </svg>
  )
}
