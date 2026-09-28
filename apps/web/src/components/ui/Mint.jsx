import { shortKey } from '@basketfun/solana'

/** Shortened mint / address with the full value on hover. */
export default function Mint({ mint }) {
  return (
    <span className="mint mono" title={mint}>
      {shortKey(mint)}
    </span>
  )
}
