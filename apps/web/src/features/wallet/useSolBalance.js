import { useCallback, useEffect, useState } from 'react'
import { PublicKey } from '@solana/web3.js'
import { getConnection } from '../../lib/chain.js'

/** SOL balance (lamports) of `address` on the site's chain, refreshed every `everyMs`. */
export function useSolBalance(address, everyMs = 15_000) {
  const [lamports, setLamports] = useState(null)
  const refresh = useCallback(async () => {
    if (!address) return setLamports(null)
    try {
      setLamports(await getConnection().getBalance(new PublicKey(address)))
    } catch {
      setLamports(null)
    }
  }, [address])
  useEffect(() => {
    refresh()
    const t = setInterval(refresh, everyMs)
    return () => clearInterval(t)
  }, [refresh, everyMs])
  return [lamports, refresh]
}
