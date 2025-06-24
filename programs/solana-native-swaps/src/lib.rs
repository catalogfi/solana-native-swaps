use anchor_lang::{prelude::*, solana_program::hash, system_program};

declare_id!("6eksgdCnSjUaGQWZ6iYvauv1qzvYPF33RTGTM1ZuyENx");

/// The size of Anchor's internal discriminator in a PDA's memory
const ANCHOR_DISCRIMINATOR: usize = 8;

#[program]
pub mod solana_native_swaps {
    use super::*;

    /// Initiates the atomic swap. Funds are transferred from the initiator to the token vault.
    /// As such, the initiator's signature is required for this instruction.
    /// `amount_lamports` represents the quantity of native SOL to be transferred
    /// through this atomic swap in base units (aka lamports).  
    /// E.g: A quantity of 1 SOL must be provided as 1,000,000,000.
    /// `expires_in_slots` represents the number of slots (1 slot = 400ms) after
    /// which (non-instant) refunds are allowed.
    pub fn initiate(
        ctx: Context<Initiate>,
        amount_lamports: u64,
        expires_in_slots: u64,
        redeemer: Pubkey,
        secret_hash: [u8; 32],
    ) -> Result<()> {
        let transfer_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.initiator.to_account_info(),
                to: ctx.accounts.swap_account.to_account_info(),
            },
        );
        system_program::transfer(transfer_context, amount_lamports)?;

        *ctx.accounts.swap_account = SwapAccount {
            amount_lamports,
            expiry_slot: Clock::get()?.slot + expires_in_slots,
            initiator: ctx.accounts.initiator.key(),
            redeemer,
            secret_hash,
        };

        emit!(Initiated {
            swap_amount: amount_lamports,
            expires_in_slots,
            initiator: ctx.accounts.initiator.key(),
            redeemer,
            secret_hash,
        });

        Ok(())
    }

    /// Funds are transferred to the redeemer. This instruction does not require any signatures.
    pub fn redeem(ctx: Context<Redeem>, secret: [u8; 32]) -> Result<()> {
        require!(
            hash::hash(&secret).to_bytes() == ctx.accounts.swap_account.secret_hash,
            SwapError::InvalidSecret
        );

        let swap_amount = ctx.accounts.swap_account.amount_lamports;
        ctx.accounts.swap_account.sub_lamports(swap_amount)?;
        ctx.accounts.redeemer.add_lamports(swap_amount)?;

        emit!(Redeemed {
            initiator: ctx.accounts.swap_account.initiator,
            secret,
        });

        Ok(())
    }

    /// Funds are returned to the initiator, given that no redeems have occured
    /// and the expiry slot has been reached.
    /// This instruction does not require any signatures.
    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        let expiry_slot = ctx.accounts.swap_account.expiry_slot;
        let current_slot = Clock::get()?.slot;
        require!(current_slot > expiry_slot, SwapError::RefundBeforeExpiry);

        let swap_amount = ctx.accounts.swap_account.amount_lamports;
        ctx.accounts.swap_account.sub_lamports(swap_amount)?;
        ctx.accounts.initiator.add_lamports(swap_amount)?;

        emit!(Refunded {
            initiator: ctx.accounts.swap_account.initiator,
            secret_hash: ctx.accounts.swap_account.secret_hash,
        });

        Ok(())
    }

    /// Funds are returned to the initiator, with the redeemer's consent.
    /// As such, the redeemer's signature is required for this instruction.
    /// This allows for refunds before the expiry slot.
    pub fn instant_refund(ctx: Context<InstantRefund>) -> Result<()> {
        let swap_amount = ctx.accounts.swap_account.amount_lamports;
        ctx.accounts.swap_account.sub_lamports(swap_amount)?;
        ctx.accounts.initiator.add_lamports(swap_amount)?;

        emit!(InstantRefunded {
            initiator: ctx.accounts.swap_account.initiator,
            secret_hash: ctx.accounts.swap_account.secret_hash,
        });

        Ok(())
    }
}

/// Stores the state information of the atomic swap on-chain
#[account]
#[derive(InitSpace)]
pub struct SwapAccount {
    /// The quantity of native SOL to be transferred through this atomic swap in base units (aka lamports)
    amount_lamports: u64,
    /// The exact slot after which (non-instant) refunds are allowed
    expiry_slot: u64,
    /// The initiator of the atomic swap
    initiator: Pubkey,
    /// The redeemer of the atomic swap
    redeemer: Pubkey,
    /// The secret hash associated with the atomic swap
    secret_hash: [u8; 32],
}

#[derive(Accounts)]
// The parameters must have the exact name and order as specified in the underlying function
// to avoid "seed constraint violation" errors.
// Refer: https://www.anchor-lang.com/docs/references/account-constraints#instruction-attribute
#[instruction(amount_lamports: u64, expires_in_slots: u64, redeemer: Pubkey, secret_hash: [u8; 32])]
pub struct Initiate<'info> {
    /// A PDA that maintains the on-chain state of the atomic swap throughout its lifecycle.
    /// It also serves as the "vault" for this swap, by escrowing the SOL involved in this swap.
    /// The choice of seeds ensures that any swap with equal `initiator` and
    /// `secret_hash` cannot be created until an existing one completes.
    /// This PDA will be deleted upon completion of the swap.
    #[account(
        init,
        payer = initiator,
        seeds = [b"swap_account", initiator.key().as_ref(), &secret_hash],
        bump,
        space = ANCHOR_DISCRIMINATOR + SwapAccount::INIT_SPACE,
    )]
    pub swap_account: Account<'info, SwapAccount>,

    /// The initiator of the atomic swap. They must sign this transaction.
    #[account(mut)]
    pub initiator: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    /// The PDA holding the state information of the atomic swap.
    /// Will be closed upon successful execution and the resulting rent
    /// will be transferred to the initiator.
    #[account(mut, close = initiator)]
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: Verifying the initiator.  
    /// This is included here for the PDA rent refund using the `close` attribute above.
    #[account(mut, address = swap_account.initiator @ SwapError::InvalidInitiator)]
    pub initiator: AccountInfo<'info>,

    /// CHECK: Verifying the redeemer
    #[account(mut, address = swap_account.redeemer @ SwapError::InvalidRedeemer)]
    pub redeemer: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct Refund<'info> {
    /// The PDA holding the state information of the atomic swap.
    /// Will be closed upon successful execution and the resulting rent
    /// will be transferred to the initiator.
    #[account(mut, close = initiator)]
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: Verifying the initiator.
    /// This is included here for the PDA rent refund using the `close` attribute above.
    #[account(mut, address = swap_account.initiator @ SwapError::InvalidInitiator)]
    pub initiator: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct InstantRefund<'info> {
    /// The PDA holding the state information of the atomic swap.
    /// Will be closed upon successful execution and the resulting rent
    /// will be transferred to the initiator.
    #[account(mut, close = initiator)]
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: Verifying the initiator.
    /// This is included here for the PDA rent refund using the `close` attribute above.
    #[account(mut, address = swap_account.initiator @ SwapError::InvalidInitiator)]
    pub initiator: AccountInfo<'info>,

    /// CHECK: Verifying the redeemer. Redeemer must sign this transaction.
    #[account(address = swap_account.redeemer @ SwapError::InvalidRedeemer)]
    pub redeemer: Signer<'info>,
}

/// Represents the initiated state of the swap where the initiator has deposited funds into the vault
#[event]
pub struct Initiated {
    /// The quantity of native SOL transferred through this atomic swap in base units (aka lamports).  
    /// E.g: A quantity of 1 SOL will be represented as 1,000,000,000.
    pub swap_amount: u64,
    /// `expires_in_slots` represents the number of slots (1 slot = 400ms) after which
    /// (non-instant) refunds are allowed
    pub expires_in_slots: u64,
    pub initiator: Pubkey,
    pub redeemer: Pubkey,
    pub secret_hash: [u8; 32],
}
/// Represents the redeemed state of the swap, where the redeemer has withdrawn funds from the vault
#[event]
pub struct Redeemed {
    pub initiator: Pubkey,
    pub secret: [u8; 32],
}
/// Represents the refund state of the swap, where the initiator has withdrawn funds from the vault past expiry
#[event]
pub struct Refunded {
    pub initiator: Pubkey,
    pub secret_hash: [u8; 32],
}
/// Represents the instant refund state of the swap, where the initiator has withdrawn funds the vault
/// with the redeemer's consent
#[event]
pub struct InstantRefunded {
    pub initiator: Pubkey,
    pub secret_hash: [u8; 32],
}

#[error_code]
pub enum SwapError {
    #[msg("The provided initiator is not the original initiator of this swap")]
    InvalidInitiator,

    #[msg("The provided redeemer is not the original redeemer of this swap")]
    InvalidRedeemer,

    #[msg("The provided secret does not correspond to the secret hash of this swap")]
    InvalidSecret,

    #[msg("Attempt to perform a refund before expiry time")]
    RefundBeforeExpiry,
}
