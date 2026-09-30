//! Program errors. Exactly one `#[error_code]` block is allowed per program
//! in Anchor 1.x; every error lives here. The first block is the enum the
//! confirmation doc specifies (§5), in its order; the second block is
//! implementation detail that the spec leaves to the program.

use anchor_lang::prelude::*;

#[error_code]
pub enum BasketError {
    #[msg("Weights must sum to exactly 10,000 basis points")]
    WeightsMustSumToTotal,
    #[msg("Unknown basket type")]
    InvalidBasketType,
    #[msg("Mint is not on the whitelist")]
    MintNotWhitelisted,
    #[msg("Duplicate mint in book")]
    DuplicateMint,
    #[msg("Too many assets for this basket type")]
    TooManyAssets,
    #[msg("Token program does not own this mint")]
    BadTokenProgram,
    #[msg("Attestation signature missing or does not cover this payload")]
    BadSignature,
    #[msg("Name longer than 32 bytes")]
    NameTooLong,
    #[msg("Symbol longer than 10 bytes")]
    SymbolTooLong,
    #[msg("Fee outside the allowed band")]
    FeeOutOfBounds,
    #[msg("Sleeve ratio outside the profile band")]
    SleeveOutOfBand,
    #[msg("Mint gate is closed")]
    MintGateClosed,
    #[msg("Below the minimum size")]
    BelowMinimum,
    #[msg("Basket book is not complete yet")]
    BasketIncomplete,
    #[msg("This basket type cannot be changed")]
    ImmutableBasket,
    #[msg("Signer is not allowed to do this")]
    Unauthorized,
    #[msg("Timelock has not elapsed")]
    TimelockActive,
    #[msg("Turnover cap exceeded for this window")]
    TurnoverExceeded,
    #[msg("Rebalance window is closed")]
    RebalanceWindowClosed,
    #[msg("Slippage exceeded")]
    SlippageExceeded,
    #[msg("Mint is not in the target book")]
    OffBook,
    #[msg("Component account is frozen; pass a FrozenClaim account for it")]
    ComponentFrozen,
    #[msg("Nothing to crystallize")]
    NothingToCrystallize,
    #[msg("Protocol is paused")]
    Paused,

    // ---- implementation detail below this line ----
    #[msg("URI longer than 200 bytes")]
    UriTooLong,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("Invalid argument")]
    InvalidArgument,
    #[msg("Book hash does not match the signed payload")]
    BookHashMismatch,
    #[msg("Basket is already seeded")]
    AlreadySeeded,
    #[msg("Basket has not been seeded")]
    NotSeeded,
    #[msg("Wrong number of component accounts")]
    ComponentCountMismatch,
    #[msg("Component account does not belong to this basket")]
    ComponentMismatch,
    #[msg("Pool account does not match the basket")]
    PoolMismatch,
    #[msg("Gate or schedule parameters are invalid")]
    InvalidGate,
    #[msg("Whitelist is full")]
    WhitelistFull,
    #[msg("Zero amount")]
    ZeroAmount,
    #[msg("Basket still has supply or recent activity")]
    NotClosable,
    #[msg("Lock has not expired")]
    LockActive,
    #[msg("No rebalance in progress")]
    NoRebalance,
    #[msg("A redemption is still pending for this wallet")]
    RedemptionPending,
    #[msg("Crystallization period has not elapsed")]
    CrystallizeTooSoon,
    #[msg("A rebalance is in progress")]
    RebalanceActive,
    #[msg("Position vault is not empty")]
    PositionNotEmpty,
    #[msg("Positions leaving the book must be closed before finalizing")]
    PositionsNotClosed,
    #[msg("Sell exceeds the per-position cap for this rebalance")]
    SellCapExceeded,
    #[msg("Swap program not allowed")]
    SwapProgramNotAllowed,
    #[msg("Swap touches a basket account other than the declared vaults")]
    UnexpectedAccountInSwap,
    #[msg("Sleeve SOL leg does not match the launch bin price")]
    LaunchPriceMismatch,
    #[msg("Backstop is not in the state this instruction needs")]
    BackstopState,
    #[msg("Bin array account missing or does not belong to this pool")]
    BinArrayMismatch,
    #[msg("Position account does not match the basket")]
    PositionMismatch,
    #[msg("Active bin is still inside the trigger band")]
    RecenterNotNeeded,
    #[msg("Re-centred too recently")]
    RecenterTooSoon,
    #[msg("$BSKT mint is not set or is already set")]
    BsktMintState,
    #[msg("Prize epoch has not ended or was already paid")]
    PrizeEpoch,
    #[msg("Recipient is not eligible for a prize")]
    PrizeIneligible,
    #[msg("Fee settlement swap did not land where expected")]
    SettleMismatch,
}
