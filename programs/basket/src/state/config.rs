//! `Config` — one per deployment. Admin (Squads), keeper set, treasury and
//! team wallets, fee split and bounds, sleeve profiles, DLMM pool presets,
//! deposit, mint-window bounds, prize parameters, pause flag (§2, D17, D18,
//! change order §2–§5).
//!
//! Everything in `FeeDefaults`, `SleeveProfile`, `PoolPreset` and
//! `ManagedRules` is copied into a basket at creation; editing `Config` only
//! affects new baskets (§4). "Timelocked" admin changes are timelocked at
//! the Squads level (D14/D18); the program only enforces what it can, e.g.
//! `bskt_mint` is set once and never changed.

use anchor_lang::prelude::*;

use crate::constants::*;

#[account]
#[derive(InitSpace, Debug)]
pub struct Config {
    pub bump: u8,
    /// 0 mainnet, 1 devnet, 2 localnet. Part of the tier-attestation domain.
    pub cluster: u8,
    pub paused: bool,
    pub admin: Pubkey,
    /// Squads vault that receives creation fees and close-time sleeve dust.
    pub treasury: Pubkey,
    /// Receives the team fee line (change order §4). Squads vault on mainnet.
    pub team_wallet: Pubkey,
    /// May edit the whitelist. Usually the admin, can be a hot key.
    pub whitelist_authority: Pubkey,
    #[max_len(MAX_KEEPERS)]
    pub keepers: Vec<Pubkey>,
    pub fees: FeeDefaults,
    pub sleeve: SleeveDefaults,
    pub pools: PoolDefaults,
    pub managed: ManagedRules,
    pub rebalance_window_s: i64,
    pub close_idle_s: i64,
    /// Programs `execute_swap` / `settle_fees` / `execute_buyback` may CPI
    /// into (Jupiter v6 on mainnet). Empty = no swaps possible.
    #[max_len(MAX_SWAP_PROGRAMS)]
    pub swap_programs: Vec<Pubkey>,
    /// Slack on the weight-derived per-position sell cap during a rebalance.
    pub rebalance_tolerance_bps: u16,
    /// Deployer's refundable launch deposit (change order §2.2).
    pub deposit_lamports: u64,
    pub mint_window: MintWindow,
    /// `MintGate::Open` is only accepted while this is set (ships false).
    pub allow_open_gate: bool,
    /// Set once by `set_bskt_mint`; `execute_buyback` refuses while `None`.
    pub bskt_mint: Option<Pubkey>,
    pub prizes: PrizeParams,
    /// Creator fee line (bps of every fee event) by attested tier; tier 0 is
    /// the base. The other three lines share the remainder pro rata.
    pub creator_tier_bps: [u16; TIER_COUNT],
}

impl Config {
    pub fn is_keeper(&self, key: &Pubkey) -> bool {
        self.keepers.iter().any(|k| k == key)
    }

    pub fn allows_swap_program(&self, key: &Pubkey) -> bool {
        self.swap_programs.iter().any(|k| k == key)
    }

    pub fn creator_bps_for_tier(&self, tier: u8) -> u16 {
        self.creator_tier_bps[(tier as usize).min(TIER_COUNT - 1)]
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, PartialEq, Eq)]
pub struct FeeDefaults {
    pub mint_fee_bps: u16,
    pub redeem_fee_bps: u16,
    /// Four lines summing to `BPS_TOTAL` (change order §4).
    pub creator_split_bps: u16,
    pub buyback_split_bps: u16,
    pub team_split_bps: u16,
    pub prize_split_bps: u16,
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
            creator_split_bps: DEFAULT_CREATOR_SPLIT_BPS,
            buyback_split_bps: DEFAULT_BUYBACK_SPLIT_BPS,
            team_split_bps: DEFAULT_TEAM_SPLIT_BPS,
            prize_split_bps: DEFAULT_PRIZE_SPLIT_BPS,
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
        self.creator_split_bps as u32
            + self.buyback_split_bps as u32
            + self.team_split_bps as u32
            + self.prize_split_bps as u32
            == BPS_TOTAL as u32
            && self.mint_fee_bps < BPS_TOTAL
            && self.redeem_fee_bps < BPS_TOTAL
            && self.perf_fee_default_bps <= self.perf_fee_max_bps
            && self.perf_fee_max_bps < BPS_TOTAL
            && self.mgmt_fee_bps_per_year < BPS_TOTAL
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, PartialEq, Eq)]
pub struct SleeveProfile {
    /// Default `r` after the step phase.
    pub default_r_bps: u16,
    pub min_r_bps: u16,
    pub max_r_bps: u16,
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
                SleeveProfile { default_r_bps: 700, min_r_bps: 500, max_r_bps: 1_000 },
                // Mirror
                SleeveProfile { default_r_bps: 1_000, min_r_bps: 500, max_r_bps: 1_500 },
                // Strategy
                SleeveProfile { default_r_bps: 1_200, min_r_bps: 800, max_r_bps: 2_000 },
                // Managed
                SleeveProfile { default_r_bps: 1_200, min_r_bps: 800, max_r_bps: 2_000 },
                // Fixed, meme/sector
                SleeveProfile { default_r_bps: 2_000, min_r_bps: 1_500, max_r_bps: 2_500 },
            ],
        }
    }
}

impl SleeveDefaults {
    pub fn validate(&self) -> bool {
        self.step_r_bps > 0
            && self.step_r_bps < BPS_TOTAL
            && self.profiles.iter().all(|p| {
                p.min_r_bps > 0 && p.min_r_bps <= p.default_r_bps && p.default_r_bps <= p.max_r_bps && p.max_r_bps < BPS_TOTAL
            })
    }
}

/// DLMM pool preset per profile (progress §6.4a). The creator cannot
/// change these; tune on devnet.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, Default, PartialEq, Eq)]
pub struct PoolPreset {
    pub bin_step: u16,
    /// Flat base fee of the pool.
    pub base_fee_bps: u16,
    /// `tight` = `[active − w, active + w]`.
    pub tight_half_width_bins: u16,
    /// Backstop target half-width; the placed range is whole bin arrays
    /// around the launch bin, at most `backstop_max_arrays` of them.
    pub backstop_half_width_bins: u16,
    pub backstop_max_arrays: u8,
    /// Keeper-side: consecutive out-of-range checks before `reset_backstop`.
    pub reset_confirm_checks: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, PartialEq, Eq)]
pub struct PoolDefaults {
    /// Indexed by `PROFILE_*`.
    pub presets: [PoolPreset; PROFILE_COUNT],
    /// Re-centre once the active bin is past this share of the half-width.
    pub recenter_trigger_bps: u16,
    /// Minimum interval between triggered re-centres of one basket.
    pub recenter_min_interval_s: i64,
    /// Share of each sleeve deposit that goes to the backstop.
    pub backstop_slice_bps: u16,
}

impl Default for PoolDefaults {
    fn default() -> Self {
        // Half-widths in bins = ln(1 + pct) / ln(1 + bin_step / 10_000).
        let index = PoolPreset {
            bin_step: 25,
            base_fee_bps: 25,
            tight_half_width_bins: 31, // ±8 %
            backstop_half_width_bins: 135, // ±40 %
            backstop_max_arrays: MAX_BACKSTOP_ARRAYS,
            reset_confirm_checks: 3,
        };
        let active = PoolPreset {
            bin_step: 50,
            base_fee_bps: 50,
            tight_half_width_bins: 23, // ±12 %
            backstop_half_width_bins: 81, // ±50 %
            backstop_max_arrays: MAX_BACKSTOP_ARRAYS,
            reset_confirm_checks: 3,
        };
        let meme = PoolPreset {
            bin_step: 100,
            base_fee_bps: 100,
            tight_half_width_bins: 14, // ±15 %
            backstop_half_width_bins: 53, // ±70 %
            backstop_max_arrays: MAX_BACKSTOP_ARRAYS,
            reset_confirm_checks: 1,
        };
        Self {
            // Managed is not named in the decision table; it takes the
            // Mirror/Strategy preset (creator-run active books).
            presets: [index, active, active, active, meme],
            recenter_trigger_bps: DEFAULT_RECENTER_TRIGGER_BPS,
            recenter_min_interval_s: DEFAULT_RECENTER_MIN_INTERVAL_S,
            backstop_slice_bps: DEFAULT_BACKSTOP_SLICE_BPS,
        }
    }
}

impl PoolDefaults {
    pub fn validate(&self) -> bool {
        self.recenter_trigger_bps > 0
            && self.recenter_trigger_bps <= BPS_TOTAL
            && self.recenter_min_interval_s >= 0
            && self.backstop_slice_bps < BPS_TOTAL
            && self.presets.iter().all(|p| {
                p.bin_step > 0
                    && p.bin_step <= crate::dlmm::MAX_BIN_STEP
                    && p.base_fee_bps > 0
                    && p.base_fee_bps <= 1_000
                    && p.tight_half_width_bins > 0
                    && (2 * p.tight_half_width_bins as i32 + 1) <= crate::dlmm::DEFAULT_BIN_PER_POSITION
                    && p.backstop_half_width_bins as i32 > p.tight_half_width_bins as i32
                    && p.backstop_max_arrays >= 1
                    && p.backstop_max_arrays <= MAX_BACKSTOP_ARRAYS
                    && p.reset_confirm_checks >= 1
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

/// Mint window bounds (change order §2.3). A creator's `WindowUntil` must
/// close between `min_s` and `max_s` after creation; `default_s` is what
/// the frontend pre-fills.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, PartialEq, Eq)]
pub struct MintWindow {
    pub default_s: i64,
    pub min_s: i64,
    pub max_s: i64,
}

impl Default for MintWindow {
    fn default() -> Self {
        Self { default_s: DEFAULT_MINT_WINDOW_S, min_s: DEFAULT_MIN_MINT_WINDOW_S, max_s: DEFAULT_MAX_MINT_WINDOW_S }
    }
}

impl MintWindow {
    pub fn validate(&self) -> bool {
        self.min_s > 0 && self.min_s <= self.default_s && self.default_s <= self.max_s
    }
}

/// Deployer prize pool (change order §5, Q9).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, PartialEq, Eq)]
pub struct PrizeParams {
    pub epoch_s: i64,
    /// Epoch 0 starts here (set at `initialize_config`).
    pub epoch_anchor_ts: i64,
    pub max_recipients: u8,
    /// Rank → share of the epoch's payout; the first `max_recipients` must
    /// sum to `BPS_TOTAL`.
    pub split_bps: [u16; MAX_PRIZE_RECIPIENTS],
    /// A recipient's basket must have claimed this much pool-fee SOL in the epoch.
    pub min_epoch_pool_fees: u64,
}

impl Default for PrizeParams {
    fn default() -> Self {
        Self {
            epoch_s: DEFAULT_PRIZE_EPOCH_S,
            epoch_anchor_ts: 0,
            max_recipients: DEFAULT_PRIZE_MAX_RECIPIENTS,
            split_bps: DEFAULT_PRIZE_RANK_SPLIT_BPS,
            min_epoch_pool_fees: DEFAULT_MIN_EPOCH_POOL_FEES,
        }
    }
}

impl PrizeParams {
    pub fn validate(&self) -> bool {
        let n = self.max_recipients as usize;
        self.epoch_s > 0
            && n >= 1
            && n <= MAX_PRIZE_RECIPIENTS
            && self.split_bps[..n].iter().map(|b| *b as u32).sum::<u32>() == BPS_TOTAL as u32
            && self.split_bps[n..].iter().all(|b| *b == 0)
    }

    pub fn epoch_at(&self, now: i64) -> u64 {
        if now < self.epoch_anchor_ts {
            return 0;
        }
        ((now - self.epoch_anchor_ts) / self.epoch_s) as u64
    }
}

/// Everything `initialize_config` / `update_config` accept. `None` keeps the
/// current (or default) value.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct ConfigUpdate {
    pub admin: Option<Pubkey>,
    pub treasury: Option<Pubkey>,
    pub team_wallet: Option<Pubkey>,
    pub whitelist_authority: Option<Pubkey>,
    pub fees: Option<FeeDefaults>,
    pub sleeve: Option<SleeveDefaults>,
    pub pools: Option<PoolDefaults>,
    pub managed: Option<ManagedRules>,
    pub rebalance_window_s: Option<i64>,
    pub close_idle_s: Option<i64>,
    pub swap_programs: Option<Vec<Pubkey>>,
    pub rebalance_tolerance_bps: Option<u16>,
    pub deposit_lamports: Option<u64>,
    pub mint_window: Option<MintWindow>,
    pub allow_open_gate: Option<bool>,
    pub prizes: Option<PrizeParams>,
    pub creator_tier_bps: Option<[u16; TIER_COUNT]>,
}
