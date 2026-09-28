//! `Config` — one per deployment. Admin (Squads), keeper set, treasury, fee
//! bounds and defaults, sleeve profiles, pause flag (§2, D17, D18).
//!
//! Everything in `FeeDefaults`, `SleeveProfile` and `ManagedRules` is copied
//! into a basket at creation; editing `Config` only affects new baskets (§4).

use anchor_lang::prelude::*;

use crate::constants::*;

#[account]
#[derive(InitSpace, Debug)]
pub struct Config {
    pub bump: u8,
    /// 0 mainnet, 1 devnet, 2 localnet. Part of the ed25519 domain (D19).
    pub cluster: u8,
    pub paused: bool,
    pub admin: Pubkey,
    /// Squads vault that receives the protocol fee line and creation fees.
    pub treasury: Pubkey,
    /// May edit the whitelist. Usually the admin, can be a hot key.
    pub whitelist_authority: Pubkey,
    #[max_len(MAX_KEEPERS)]
    pub keepers: Vec<Pubkey>,
    pub fees: FeeDefaults,
    pub sleeve: SleeveDefaults,
    pub managed: ManagedRules,
    pub rebalance_window_s: i64,
    pub close_idle_s: i64,
    pub graduation_lamports: u64,
    pub scheduler: LaunchScheduler,
}

impl Config {
    pub fn is_keeper(&self, key: &Pubkey) -> bool {
        self.keepers.iter().any(|k| k == key)
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, PartialEq, Eq)]
pub struct FeeDefaults {
    pub mint_fee_bps: u16,
    pub redeem_fee_bps: u16,
    pub holder_split_bps: u16,
    pub creator_split_bps: u16,
    pub protocol_split_bps: u16,
    pub mgmt_fee_bps_per_year: u16,
    pub perf_fee_default_bps: u16,
    pub perf_fee_max_bps: u16,
    pub creation_fee_lamports: u64,
    pub min_seed_lamports: u64,
    pub min_mint_lamports: u64,
}

impl Default for FeeDefaults {
    fn default() -> Self {
        Self {
            mint_fee_bps: DEFAULT_MINT_FEE_BPS,
            redeem_fee_bps: DEFAULT_REDEEM_FEE_BPS,
            holder_split_bps: DEFAULT_HOLDER_SPLIT_BPS,
            creator_split_bps: DEFAULT_CREATOR_SPLIT_BPS,
            protocol_split_bps: DEFAULT_PROTOCOL_SPLIT_BPS,
            mgmt_fee_bps_per_year: DEFAULT_MGMT_FEE_BPS_PER_YEAR,
            perf_fee_default_bps: DEFAULT_PERF_FEE_BPS,
            perf_fee_max_bps: DEFAULT_PERF_FEE_MAX_BPS,
            creation_fee_lamports: DEFAULT_CREATION_FEE_LAMPORTS,
            min_seed_lamports: DEFAULT_MIN_SEED_LAMPORTS,
            min_mint_lamports: DEFAULT_MIN_MINT_LAMPORTS,
        }
    }
}

impl FeeDefaults {
    pub fn validate(&self) -> bool {
        self.holder_split_bps + self.creator_split_bps + self.protocol_split_bps == BPS_TOTAL
            && self.mint_fee_bps < BPS_TOTAL
            && self.redeem_fee_bps < BPS_TOTAL
            && self.perf_fee_default_bps <= self.perf_fee_max_bps
            && self.perf_fee_max_bps < BPS_TOTAL
            && self.mgmt_fee_bps_per_year < BPS_TOTAL
    }
}

/// Pool price range relative to the seed price (§3).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, PartialEq, Eq)]
pub enum PriceRange {
    /// `[0.5×, 8×]`
    Bounded,
    /// `[0.5×, ∞)`
    FloorOnly,
    /// cp-amm `MIN_SQRT_PRICE..MAX_SQRT_PRICE`
    Full,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, PartialEq, Eq)]
pub struct SleeveProfile {
    /// Default `r` after the step phase.
    pub default_r_bps: u16,
    pub min_r_bps: u16,
    pub max_r_bps: u16,
    /// Pool swap fee.
    pub pool_fee_bps: u16,
    pub price_range: PriceRange,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, PartialEq, Eq)]
pub struct SleeveDefaults {
    /// `r` during the step phase (D5: 25%).
    pub step_r_bps: u16,
    /// Pool SOL side at which the step phase ends (D5: 20 SOL).
    pub step_threshold_lamports: u64,
    /// Indexed by `PROFILE_*`.
    pub profiles: [SleeveProfile; PROFILE_COUNT],
}

impl Default for SleeveDefaults {
    fn default() -> Self {
        Self {
            step_r_bps: DEFAULT_STEP_R_BPS,
            step_threshold_lamports: DEFAULT_STEP_THRESHOLD_LAMPORTS,
            profiles: [
                // Fixed, majors/index
                SleeveProfile {
                    default_r_bps: 700,
                    min_r_bps: 500,
                    max_r_bps: 1_000,
                    pool_fee_bps: 30,
                    price_range: PriceRange::Bounded,
                },
                // Mirror
                SleeveProfile {
                    default_r_bps: 1_000,
                    min_r_bps: 500,
                    max_r_bps: 1_500,
                    pool_fee_bps: 30,
                    price_range: PriceRange::Bounded,
                },
                // Strategy
                SleeveProfile {
                    default_r_bps: 1_200,
                    min_r_bps: 800,
                    max_r_bps: 2_000,
                    pool_fee_bps: 100,
                    price_range: PriceRange::FloorOnly,
                },
                // Managed
                SleeveProfile {
                    default_r_bps: 1_200,
                    min_r_bps: 800,
                    max_r_bps: 2_000,
                    pool_fee_bps: 100,
                    price_range: PriceRange::FloorOnly,
                },
                // Fixed, meme/sector
                SleeveProfile {
                    default_r_bps: 2_000,
                    min_r_bps: 1_500,
                    max_r_bps: 2_500,
                    pool_fee_bps: 100,
                    price_range: PriceRange::Full,
                },
            ],
        }
    }
}

impl SleeveDefaults {
    pub fn validate(&self) -> bool {
        self.step_r_bps > 0
            && self.step_r_bps < BPS_TOTAL
            && self.profiles.iter().all(|p| {
                p.min_r_bps > 0
                    && p.min_r_bps <= p.default_r_bps
                    && p.default_r_bps <= p.max_r_bps
                    && p.max_r_bps < BPS_TOTAL
                    && p.pool_fee_bps >= 1
                    && p.pool_fee_bps <= 9_900
            })
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, PartialEq, Eq)]
pub struct ManagedRules {
    pub timelock_s: i64,
    pub turnover_cap_bps: u16,
    pub turnover_window_s: i64,
    pub crystallize_period_s: i64,
}

impl Default for ManagedRules {
    fn default() -> Self {
        Self {
            timelock_s: DEFAULT_MANAGED_TIMELOCK_S,
            turnover_cap_bps: DEFAULT_TURNOVER_CAP_BPS,
            turnover_window_s: DEFAULT_TURNOVER_WINDOW_S,
            crystallize_period_s: DEFAULT_CRYSTALLIZE_PERIOD_S,
        }
    }
}

/// D13 launch sniper tax: DAMM v2 linear fee-time scheduler, on for
/// Window/Closed gates only. Fee starts at `cliff_bps` and decays to the
/// profile's pool fee in `periods` steps of `period_s`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, PartialEq, Eq)]
pub struct LaunchScheduler {
    pub cliff_bps: u16,
    pub periods: u16,
    pub period_s: u64,
}

impl Default for LaunchScheduler {
    fn default() -> Self {
        Self {
            cliff_bps: DEFAULT_SCHEDULER_CLIFF_BPS,
            periods: DEFAULT_SCHEDULER_PERIODS,
            period_s: DEFAULT_SCHEDULER_PERIOD_S,
        }
    }
}

/// Everything `initialize_config` / `update_config` accept. `None` keeps the
/// current (or default) value.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct ConfigUpdate {
    pub admin: Option<Pubkey>,
    pub treasury: Option<Pubkey>,
    pub whitelist_authority: Option<Pubkey>,
    pub fees: Option<FeeDefaults>,
    pub sleeve: Option<SleeveDefaults>,
    pub managed: Option<ManagedRules>,
    pub rebalance_window_s: Option<i64>,
    pub close_idle_s: Option<i64>,
    pub graduation_lamports: Option<u64>,
    pub scheduler: Option<LaunchScheduler>,
}
