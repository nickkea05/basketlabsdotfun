//! Client-side builders and readers for the Meteora DLMM (`lb_clmm`)
//! fixture: pool creation, bin arrays, PDA positions, add / remove / claim /
//! close, swaps, and the position-amount arithmetic the program's `PoolView`
//! does on-chain (mirrored here so tests can check NAV bookkeeping).

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use basket::dlmm::{self, lb_clmm};
use solana_keypair::Keypair;
use solana_signer::Signer;

use super::{Env, LAMPORTS, WSOL};

pub const LB_CLMM_ID: Pubkey = dlmm::LB_CLMM_ID;
pub const MEMO_ID: Pubkey = dlmm::MEMO_PROGRAM_ID;
pub const RENT_SYSVAR: Pubkey = pubkey!("SysvarRent111111111111111111111111111111111");

/// A pool with `token_x` = `mint_x` and `token_y` = `mint_y` (wSOL for
/// basket pools).
#[derive(Clone, Copy, Debug)]
pub struct DlmmPool {
    pub lb_pair: Pubkey,
    pub mint_x: Pubkey,
    pub mint_y: Pubkey,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
    pub oracle: Pubkey,
    pub bin_step: u16,
}

/// Token amounts a position holds, derived from its bin shares and the bin
/// arrays exactly as the program does (X rounded up, Y down).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PositionAmounts {
    pub lower_bin_id: i32,
    pub upper_bin_id: i32,
    pub amount_x: u64,
    pub amount_y: u64,
}

impl DlmmPool {
    pub fn load(&self, env: &Env) -> lb_clmm::accounts::LbPair {
        let acc = env.account(&self.lb_pair).expect("lb_pair missing");
        assert_eq!(acc.owner, LB_CLMM_ID);
        bytemuck::pod_read_unaligned(&acc.data[8..8 + std::mem::size_of::<lb_clmm::accounts::LbPair>()])
    }

    pub fn exists(&self, env: &Env) -> bool {
        env.account(&self.lb_pair).map(|a| a.owner == LB_CLMM_ID).unwrap_or(false)
    }

    pub fn active_id(&self, env: &Env) -> i32 {
        self.load(env).active_id
    }

    pub fn reserves(&self, env: &Env) -> (u64, u64) {
        (env.token_amount(&self.reserve_x), env.token_amount(&self.reserve_y))
    }

    pub fn bin_array(&self, index: i64) -> Pubkey {
        dlmm::bin_array(&self.lb_pair, index)
    }

    /// Bin arrays (writable metas) covering `[lower, upper]`.
    pub fn bin_array_metas(&self, lower: i32, upper: i32) -> Vec<AccountMeta> {
        let (lo, hi) = dlmm::bin_array_range(lower, upper);
        (lo..=hi).map(|i| AccountMeta::new(self.bin_array(i), false)).collect()
    }

    pub fn load_bin_array(&self, env: &Env, index: i64) -> Option<lb_clmm::accounts::BinArray> {
        let acc = env.account(&self.bin_array(index))?;
        if acc.data.len() < 8 + std::mem::size_of::<lb_clmm::accounts::BinArray>() {
            return None;
        }
        Some(bytemuck::pod_read_unaligned(
            &acc.data[8..8 + std::mem::size_of::<lb_clmm::accounts::BinArray>()],
        ))
    }

    /// Sum of (amount_x, amount_y) across bins `[lower, upper]`.
    pub fn bin_amounts(&self, env: &Env, lower: i32, upper: i32) -> (u64, u64) {
        let mut x = 0u64;
        let mut y = 0u64;
        for id in lower..=upper {
            let idx = dlmm::bin_array_index(id);
            if let Some(arr) = self.load_bin_array(env, idx) {
                let offset = (id - (idx as i32) * dlmm::MAX_BIN_PER_ARRAY) as usize;
                x += arr.bins[offset].amount_x;
                y += arr.bins[offset].amount_y;
            }
        }
        (x, y)
    }

    /// Price of bin `id` in Q64.64 lamports (Y base units) per X base unit.
    pub fn price_q64(&self, id: i32) -> u128 {
        dlmm::bin_price_q64(id, self.bin_step).unwrap()
    }

    /// Mirror of `PoolView::read_position`.
    pub fn position_amounts(&self, env: &Env, position: &Pubkey) -> PositionAmounts {
        let acc = env.account(position).expect("position missing");
        assert_eq!(acc.owner, LB_CLMM_ID, "not a DLMM position");
        let h = dlmm::position_header(&acc.data).expect("position header");
        assert_eq!(h.lb_pair, self.lb_pair);
        let mut out = PositionAmounts { lower_bin_id: h.lower_bin_id, upper_bin_id: h.upper_bin_id, amount_x: 0, amount_y: 0 };
        for offset in 0..(h.upper_bin_id - h.lower_bin_id + 1) {
            let share = dlmm::position_bin_share(&acc.data, offset as usize).unwrap();
            if share == 0 {
                continue;
            }
            let bin_id = h.lower_bin_id + offset;
            let idx = dlmm::bin_array_index(bin_id);
            let arr = env.account(&self.bin_array(idx)).expect("bin array missing");
            let slot = (bin_id - dlmm::array_lower_bin(idx)) as usize;
            let bin = dlmm::bin_at(&arr.data, slot).unwrap();
            if bin.liquidity_supply == 0 {
                continue;
            }
            let (x, y) = if share >= bin.liquidity_supply {
                (bin.amount_x, bin.amount_y)
            } else {
                let x = ruint::aliases::U256::from(bin.amount_x) * ruint::aliases::U256::from(share);
                let (xq, xr) = x.div_rem(ruint::aliases::U256::from(bin.liquidity_supply));
                let xq = if xr.is_zero() { xq } else { xq + ruint::aliases::U256::from(1u8) };
                let y = ruint::aliases::U256::from(bin.amount_y) * ruint::aliases::U256::from(share)
                    / ruint::aliases::U256::from(bin.liquidity_supply);
                (u64::try_from(xq).unwrap(), u64::try_from(y).unwrap())
            };
            out.amount_x += x;
            out.amount_y += y;
        }
        out
    }
}

pub fn load_position(env: &Env, position: &Pubkey) -> lb_clmm::accounts::PositionV2 {
    let acc = env.account(position).expect("position missing");
    assert_eq!(acc.owner, LB_CLMM_ID);
    bytemuck::pod_read_unaligned(&acc.data[8..8 + std::mem::size_of::<lb_clmm::accounts::PositionV2>()])
}

/// Position header (owner, range) without copying the whole account.
pub fn position_header(env: &Env, position: &Pubkey) -> dlmm::PositionHeader {
    let acc = env.account(position).expect("position missing");
    dlmm::position_header(&acc.data).expect("position header")
}

impl Env {
    /// `initialize_customizable_permissionless_lb_pair2` with X = `mint_x`,
    /// Y = wSOL, `active_id` as the starting bin, flat base fee `fee_bps`,
    /// fees collected in the input token (the spike's setting).
    pub fn create_dlmm_pool(&mut self, funder: &Keypair, mint_x: Pubkey, bin_step: u16, fee_bps: u64, active_id: i32) -> DlmmPool {
        self.create_dlmm_pool_xy(funder, mint_x, WSOL, bin_step, fee_bps, active_id, dlmm::COLLECT_FEE_MODE_INPUT_ONLY)
    }

    /// General form: any X / Y pair and collect-fee mode. The funder must
    /// already hold ATAs for both mints (created here if missing).
    #[allow(clippy::too_many_arguments)]
    pub fn create_dlmm_pool_xy(
        &mut self,
        funder: &Keypair,
        mint_x: Pubkey,
        mint_y: Pubkey,
        bin_step: u16,
        fee_bps: u64,
        active_id: i32,
        collect_fee_mode: u8,
    ) -> DlmmPool {
        let lb_pair = dlmm::customizable_lb_pair(&mint_x, &mint_y);
        let reserve_x = dlmm::reserve(&lb_pair, &mint_x);
        let reserve_y = dlmm::reserve(&lb_pair, &mint_y);
        let oracle = dlmm::oracle(&lb_pair);
        let (base_factor, base_fee_power_factor) = dlmm::base_factor_for_fee_bps(bin_step, fee_bps).expect("fee not representable");
        let user_x = self.create_ata(&funder.pubkey(), &mint_x);
        let user_y = self.create_ata(&funder.pubkey(), &mint_y);
        let ix = Instruction {
            program_id: LB_CLMM_ID,
            accounts: lb_clmm::client::accounts::InitializeCustomizablePermissionlessLbPair2 {
                lb_pair,
                bin_array_bitmap_extension: None,
                token_mint_x: mint_x,
                token_mint_y: mint_y,
                reserve_x,
                reserve_y,
                oracle,
                user_token_x: user_x,
                funder: funder.pubkey(),
                token_badge_x: None,
                token_badge_y: None,
                token_program_x: spl_token::ID,
                token_program_y: spl_token::ID,
                system_program: anchor_lang::system_program::ID,
                user_token_y: user_y,
                event_authority: dlmm::event_authority(),
                program: LB_CLMM_ID,
            }
            .to_account_metas(None),
            data: lb_clmm::client::args::InitializeCustomizablePermissionlessLbPair2 {
                params: lb_clmm::types::CustomizableParams {
                    active_id,
                    bin_step,
                    base_factor,
                    activation_type: dlmm::ACTIVATION_TYPE_TIMESTAMP,
                    has_alpha_vault: false,
                    activation_point: None,
                    creator_pool_on_off_control: false,
                    base_fee_power_factor,
                    concrete_function_type: 0,
                    collect_fee_mode,
                    padding: [0u8; 60],
                },
            }
            .data(),
        };
        let meta = self.send_ok(&[ix], funder, &[]);
        eprintln!("create pool CU {}", meta.compute_units_consumed);
        DlmmPool { lb_pair, mint_x, mint_y, reserve_x, reserve_y, oracle, bin_step }
    }

    pub fn init_bin_array_ix(&self, pool: &DlmmPool, funder: &Pubkey, index: i64) -> Instruction {
        Instruction {
            program_id: LB_CLMM_ID,
            accounts: lb_clmm::client::accounts::InitializeBinArray {
                lb_pair: pool.lb_pair,
                bin_array: pool.bin_array(index),
                funder: *funder,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: lb_clmm::client::args::InitializeBinArray { index }.data(),
        }
    }

    /// Create every missing bin array covering `[lower, upper]`; returns CU per array.
    pub fn ensure_bin_arrays(&mut self, pool: &DlmmPool, funder: &Keypair, lower: i32, upper: i32) -> Vec<u64> {
        let (lo, hi) = dlmm::bin_array_range(lower, upper);
        let mut cus = vec![];
        for i in lo..=hi {
            if self.account(&pool.bin_array(i)).is_some() {
                continue;
            }
            let ix = self.init_bin_array_ix(pool, &funder.pubkey(), i);
            let m = self.send_ok(&[ix], funder, &[]);
            cus.push(m.compute_units_consumed);
        }
        cus
    }

    /// `initialize_position_pda` owned by `owner`, base = fresh keypair. Widths
    /// above 70 are created at 70 and grown with `increase_position_length2`
    /// in ≤ `MAX_RESIZE_LENGTH` (91) steps: the runtime caps a realloc at 10 KB
    /// per instruction and each bin costs 112 bytes. Returns CU of the init.
    pub fn init_position_pda(&mut self, pool: &DlmmPool, payer: &Keypair, owner: &Keypair, lower_bin_id: i32, width: i32) -> (Pubkey, u64) {
        let init_width = width.min(dlmm::DEFAULT_BIN_PER_POSITION);
        let (position, cu) = self.init_position_pda_raw(pool, payer, owner, lower_bin_id, init_width);
        let target_upper = lower_bin_id + width - 1;
        let mut upper = lower_bin_id + init_width - 1;
        while upper < target_upper {
            upper = (upper + dlmm::MAX_RESIZE_LENGTH).min(target_upper);
            let ix = Instruction {
                program_id: LB_CLMM_ID,
                accounts: lb_clmm::client::accounts::IncreasePositionLength2 {
                    funder: payer.pubkey(),
                    lb_pair: pool.lb_pair,
                    position,
                    owner: owner.pubkey(),
                    system_program: anchor_lang::system_program::ID,
                    event_authority: dlmm::event_authority(),
                    program: LB_CLMM_ID,
                }
                .to_account_metas(None),
                data: lb_clmm::client::args::IncreasePositionLength2 { minimum_upper_bin_id: upper }.data(),
            };
            let m = self.send_ok(&[ix], payer, &[owner]);
            eprintln!("increase_position_length2 → upper {} CU {}", upper, m.compute_units_consumed);
        }
        (position, cu)
    }

    fn init_position_pda_raw(&mut self, pool: &DlmmPool, payer: &Keypair, owner: &Keypair, lower_bin_id: i32, width: i32) -> (Pubkey, u64) {
        let base = Keypair::new();
        let position = dlmm::position_pda(&pool.lb_pair, &base.pubkey(), lower_bin_id, width);
        let ix = Instruction {
            program_id: LB_CLMM_ID,
            accounts: lb_clmm::client::accounts::InitializePositionPda {
                payer: payer.pubkey(),
                base: base.pubkey(),
                position,
                lb_pair: pool.lb_pair,
                owner: owner.pubkey(),
                system_program: anchor_lang::system_program::ID,
                rent: RENT_SYSVAR,
                event_authority: dlmm::event_authority(),
                program: LB_CLMM_ID,
            }
            .to_account_metas(None),
            data: lb_clmm::client::args::InitializePositionPda { lower_bin_id, width }.data(),
        };
        let m = self.send_ok(&[ix], payer, &[&base, owner]);
        (position, m.compute_units_consumed)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_liquidity_spot_ix(
        &self,
        pool: &DlmmPool,
        position: &Pubkey,
        owner: &Pubkey,
        amount_x: u64,
        amount_y: u64,
        active_id: i32,
        min_bin_id: i32,
        max_bin_id: i32,
    ) -> Instruction {
        self.add_liquidity_ix(pool, position, owner, amount_x, amount_y, active_id, min_bin_id, max_bin_id, lb_clmm::types::StrategyType::SpotBalanced)
    }

    /// Deposit `[lower, upper]` one bin array at a time (the program runs out
    /// of heap past ~70 bins per call). Y is spread over bins < active, X over
    /// bins > active, the active bin takes both; each chunk gets its
    /// proportional slice so the whole range ends up flat. Returns CU per call.
    #[allow(clippy::too_many_arguments)]
    pub fn add_liquidity_chunked(
        &mut self,
        pool: &DlmmPool,
        position: &Pubkey,
        owner: &Keypair,
        amount_x: u64,
        amount_y: u64,
        active_id: i32,
        lower: i32,
        upper: i32,
    ) -> Vec<u64> {
        let x_bins = (upper - active_id).max(0) as u64 + u64::from(active_id >= lower && active_id <= upper);
        let y_bins = (active_id - lower).max(0) as u64 + u64::from(active_id >= lower && active_id <= upper);
        let (lo_arr, hi_arr) = dlmm::bin_array_range(lower, upper);
        let mut cus = vec![];
        let (mut x_left, mut y_left) = (amount_x, amount_y);
        for arr in lo_arr..=hi_arr {
            let c_lo = ((arr as i32) * dlmm::MAX_BIN_PER_ARRAY).max(lower);
            let c_hi = ((arr as i32 + 1) * dlmm::MAX_BIN_PER_ARRAY - 1).min(upper);
            let has_active = active_id >= c_lo && active_id <= c_hi;
            let len = (c_hi - c_lo + 1) as u64;
            let cx = if has_active { (c_hi - active_id) as u64 + 1 } else if c_lo > active_id { len } else { 0 };
            let cy = if has_active { (active_id - c_lo) as u64 + 1 } else if c_hi < active_id { len } else { 0 };
            let ax = if x_bins == 0 { 0 } else if arr == hi_arr { x_left } else { amount_x * cx / x_bins };
            let ay = if y_bins == 0 { 0 } else if arr == hi_arr { y_left } else { amount_y * cy / y_bins };
            let ax = ax.min(x_left);
            let ay = ay.min(y_left);
            if ax == 0 && ay == 0 {
                continue;
            }
            let ix = self.add_liquidity_ix(pool, position, &owner.pubkey(), ax, ay, active_id, c_lo, c_hi, lb_clmm::types::StrategyType::SpotImBalanced);
            let m = self.send_ok(&[ix], owner, &[]);
            cus.push(m.compute_units_consumed);
            x_left -= ax;
            y_left -= ay;
        }
        cus
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_liquidity_ix(
        &self,
        pool: &DlmmPool,
        position: &Pubkey,
        owner: &Pubkey,
        amount_x: u64,
        amount_y: u64,
        active_id: i32,
        min_bin_id: i32,
        max_bin_id: i32,
        strategy_type: lb_clmm::types::StrategyType,
    ) -> Instruction {
        let mut accounts = lb_clmm::client::accounts::AddLiquidityByStrategy2 {
            position: *position,
            lb_pair: pool.lb_pair,
            bin_array_bitmap_extension: None,
            user_token_x: Env::ata(owner, &pool.mint_x),
            user_token_y: Env::ata(owner, &pool.mint_y),
            reserve_x: pool.reserve_x,
            reserve_y: pool.reserve_y,
            token_x_mint: pool.mint_x,
            token_y_mint: pool.mint_y,
            sender: *owner,
            token_x_program: spl_token::ID,
            token_y_program: spl_token::ID,
            event_authority: dlmm::event_authority(),
            program: LB_CLMM_ID,
        }
        .to_account_metas(None);
        accounts.extend(pool.bin_array_metas(min_bin_id, max_bin_id));
        Instruction {
            program_id: LB_CLMM_ID,
            accounts,
            data: lb_clmm::client::args::AddLiquidityByStrategy2 {
                liquidity_parameter: lb_clmm::types::LiquidityParameterByStrategy {
                    amount_x,
                    amount_y,
                    active_id,
                    max_active_bin_slippage: 3,
                    strategy_parameters: lb_clmm::types::StrategyParameters {
                        min_bin_id,
                        max_bin_id,
                        strategy_type,
                        parameteres: [0u8; 64],
                    },
                },
                remaining_accounts_info: lb_clmm::types::RemainingAccountsInfo { slices: vec![] },
            }
            .data(),
        }
    }

    pub fn remove_liquidity_range_ix(
        &self,
        pool: &DlmmPool,
        position: &Pubkey,
        owner: &Pubkey,
        from_bin_id: i32,
        to_bin_id: i32,
        bps_to_remove: u16,
    ) -> Instruction {
        let mut accounts = lb_clmm::client::accounts::RemoveLiquidityByRange2 {
            position: *position,
            lb_pair: pool.lb_pair,
            bin_array_bitmap_extension: None,
            user_token_x: Env::ata(owner, &pool.mint_x),
            user_token_y: Env::ata(owner, &pool.mint_y),
            reserve_x: pool.reserve_x,
            reserve_y: pool.reserve_y,
            token_x_mint: pool.mint_x,
            token_y_mint: pool.mint_y,
            sender: *owner,
            token_x_program: spl_token::ID,
            token_y_program: spl_token::ID,
            memo_program: MEMO_ID,
            event_authority: dlmm::event_authority(),
            program: LB_CLMM_ID,
        }
        .to_account_metas(None);
        accounts.extend(pool.bin_array_metas(from_bin_id, to_bin_id));
        Instruction {
            program_id: LB_CLMM_ID,
            accounts,
            data: lb_clmm::client::args::RemoveLiquidityByRange2 {
                from_bin_id,
                to_bin_id,
                bps_to_remove,
                remaining_accounts_info: lb_clmm::types::RemainingAccountsInfo { slices: vec![] },
            }
            .data(),
        }
    }

    pub fn claim_fee_ix(&self, pool: &DlmmPool, position: &Pubkey, owner: &Pubkey, min_bin_id: i32, max_bin_id: i32) -> Instruction {
        let mut accounts = lb_clmm::client::accounts::ClaimFee2 {
            lb_pair: pool.lb_pair,
            position: *position,
            sender: *owner,
            reserve_x: pool.reserve_x,
            reserve_y: pool.reserve_y,
            user_token_x: Env::ata(owner, &pool.mint_x),
            user_token_y: Env::ata(owner, &pool.mint_y),
            token_x_mint: pool.mint_x,
            token_y_mint: pool.mint_y,
            token_program_x: spl_token::ID,
            token_program_y: spl_token::ID,
            memo_program: MEMO_ID,
            event_authority: dlmm::event_authority(),
            program: LB_CLMM_ID,
        }
        .to_account_metas(None);
        accounts.extend(pool.bin_array_metas(min_bin_id, max_bin_id));
        Instruction {
            program_id: LB_CLMM_ID,
            accounts,
            data: lb_clmm::client::args::ClaimFee2 {
                min_bin_id,
                max_bin_id,
                remaining_accounts_info: lb_clmm::types::RemainingAccountsInfo { slices: vec![] },
            }
            .data(),
        }
    }

    pub fn dlmm_close_position_ix(&self, position: &Pubkey, owner: &Pubkey, rent_receiver: &Pubkey) -> Instruction {
        Instruction {
            program_id: LB_CLMM_ID,
            accounts: lb_clmm::client::accounts::ClosePosition2 {
                position: *position,
                sender: *owner,
                rent_receiver: *rent_receiver,
                event_authority: dlmm::event_authority(),
                program: LB_CLMM_ID,
            }
            .to_account_metas(None),
            data: lb_clmm::client::args::ClosePosition2 {}.data(),
        }
    }

    /// `swap2` exact-in. `x_to_y` sells X for Y. Bin arrays covering
    /// `[lower, upper]` are passed as remaining accounts.
    #[allow(clippy::too_many_arguments)]
    pub fn swap_ix(&self, pool: &DlmmPool, user: &Pubkey, x_to_y: bool, amount_in: u64, min_out: u64, lower: i32, upper: i32) -> Instruction {
        let (user_in, user_out) = if x_to_y {
            (Env::ata(user, &pool.mint_x), Env::ata(user, &pool.mint_y))
        } else {
            (Env::ata(user, &pool.mint_y), Env::ata(user, &pool.mint_x))
        };
        let mut accounts = lb_clmm::client::accounts::Swap2 {
            lb_pair: pool.lb_pair,
            bin_array_bitmap_extension: None,
            reserve_x: pool.reserve_x,
            reserve_y: pool.reserve_y,
            user_token_in: user_in,
            user_token_out: user_out,
            token_x_mint: pool.mint_x,
            token_y_mint: pool.mint_y,
            oracle: pool.oracle,
            host_fee_in: None,
            user: *user,
            token_x_program: spl_token::ID,
            token_y_program: spl_token::ID,
            memo_program: MEMO_ID,
            event_authority: dlmm::event_authority(),
            program: LB_CLMM_ID,
        }
        .to_account_metas(None);
        accounts.extend(pool.bin_array_metas(lower, upper));
        Instruction {
            program_id: LB_CLMM_ID,
            accounts,
            data: lb_clmm::client::args::Swap2 {
                amount_in,
                min_amount_out: min_out,
                remaining_accounts_info: lb_clmm::types::RemainingAccountsInfo { slices: vec![] },
            }
            .data(),
        }
    }

    /// Every existing bin array of `pool` (by scanning a bin index window),
    /// as writable metas: what the basket instructions want in their tail.
    pub fn existing_bin_array_metas(&self, pool: &DlmmPool, lower: i32, upper: i32) -> Vec<AccountMeta> {
        let (lo, hi) = dlmm::bin_array_range(lower, upper);
        (lo..=hi)
            .filter(|i| self.account(&pool.bin_array(*i)).is_some())
            .map(|i| AccountMeta::new(pool.bin_array(i), false))
            .collect()
    }
}

pub fn sol(n: f64) -> u64 {
    (n * LAMPORTS as f64) as u64
}
