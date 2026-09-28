/** Placeholder artwork for baskets without a logo. `tint` is an "H S%" pair. */
export default function Orb({ tint = '0 0%', small }) {
  return <div className={`art-orb ${small ? 'small' : ''}`} style={{ '--tint': tint }} />
}
