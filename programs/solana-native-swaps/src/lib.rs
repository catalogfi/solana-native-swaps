use anchor_lang::{prelude::*, solana_program::hash, system_program};

declare_id!("6eksgdCnSjUaGQWZ6iYvauv1qzvYPF33RTGTM1ZuyENx");

/// The size of Anchor's internal discriminator in a PDA's memory
const ANCHOR_DISCRIMINATOR: usize = 8;

#[program]
pub mod solana_native_swaps {
    use super::*;

    /// Initiates the atomic swap. Funds are transferred from the funder to the swap account.
    /// `swap_amount` represents the quantity of native SOL to be transferred
    /// through this atomic swap in base units (aka lamports).  
    /// E.g: A quantity of 1 SOL must be provided as 1,000,000,000.
    /// `timelock` represents the number of slots (1 slot = 400ms) after
    /// which (non-instant) refunds are allowed.
    /// `destination_data` is an optional field, intended to hold information regarding the
    /// destination chain in the atomic swap.
    pub fn initiate(
        ctx: Context<Initiate>,
        redeemer: Pubkey,
        refundee: Pubkey,
        secret_hash: [u8; 32],
        swap_amount: u64,
        timelock: u64,
        destination_data: Option<Vec<u8>>,
    ) -> Result<()> {
        let transfer_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.funder.to_account_info(),
                to: ctx.accounts.swap_account.to_account_info(),
            },
        );
        system_program::transfer(transfer_context, swap_amount)?;

        let expiry_slot = Clock::get()?
            .slot
            .checked_add(timelock)
            .expect("timelock should not cause an overflow");
        *ctx.accounts.swap_account = SwapAccount {
            expiry_slot,
            bump: ctx.bumps.swap_account,
            rent_sponsor: ctx.accounts.rent_sponsor.key(),
            refundee,
            redeemer,
            secret_hash,
            swap_amount,
            timelock,
        };

        emit!(Initiated {
            redeemer,
            refundee,
            secret_hash,
            swap_amount,
            timelock,
            destination_data,
            funder: ctx.accounts.funder.key(),
        });

        Ok(())
    }

    /// Funds are transferred to the redeemer. This instruction does not require any signatures.
    pub fn redeem(ctx: Context<Redeem>, secret: [u8; 32]) -> Result<()> {
        let SwapAccount {
            refundee,
            redeemer,
            secret_hash,
            swap_amount,
            timelock,
            ..
        } = *ctx.accounts.swap_account;

        require!(
            hash::hash(&secret).to_bytes() == secret_hash,
            SwapError::InvalidSecret
        );

        ctx.accounts.swap_account.sub_lamports(swap_amount)?;
        ctx.accounts.redeemer.add_lamports(swap_amount)?;

        emit!(Redeemed {
            redeemer,
            refundee,
            secret,
            swap_amount,
            timelock,
        });

        Ok(())
    }

    /// The refundee obtains the funds as a refund, given that no redeems have occured
    /// and the expiry slot has been reached.
    /// This instruction does not require any signatures.
    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        let SwapAccount {
            expiry_slot,
            refundee,
            redeemer,
            secret_hash,
            swap_amount,
            timelock,
            ..
        } = *ctx.accounts.swap_account;

        let current_slot = Clock::get()?.slot;
        require!(current_slot > expiry_slot, SwapError::RefundBeforeExpiry);

        ctx.accounts.swap_account.sub_lamports(swap_amount)?;
        ctx.accounts.refundee.add_lamports(swap_amount)?;

        emit!(Refunded {
            redeemer,
            refundee,
            secret_hash,
            swap_amount,
            timelock,
        });

        Ok(())
    }

    /// Funds are refunded to the refundee, with the redeemer's consent.
    /// As such, the redeemer's signature is required for this instruction.
    /// This allows for refunds before the expiry slot.
    pub fn instant_refund(ctx: Context<InstantRefund>) -> Result<()> {
        let SwapAccount {
            refundee,
            redeemer,
            secret_hash,
            swap_amount,
            timelock,
            ..
        } = *ctx.accounts.swap_account;

        ctx.accounts.swap_account.sub_lamports(swap_amount)?;
        ctx.accounts.refundee.add_lamports(swap_amount)?;

        emit!(InstantRefunded {
            redeemer,
            refundee,
            secret_hash,
            swap_amount,
            timelock,
        });

        Ok(())
    }
}

/// Stores the state information of the atomic swap on-chain
#[account]
#[derive(InitSpace)]
pub struct SwapAccount {
    /// The exact slot after which (non-instant) refunds are allowed
    expiry_slot: u64,
    /// The bump that was used by the program to derive this PDA.
    /// Storing this makes later verifications less expensive.
    bump: u8,

    /// The redeemer of the atomic swap
    redeemer: Pubkey,
    /// The entity that is eligible to receive a refund in the atomic swap
    refundee: Pubkey,
    /// The secret hash associated with the atomic swap
    secret_hash: [u8; 32],
    /// The quantity of native SOL to be transferred through this atomic swap in base units (aka lamports)
    swap_amount: u64,
    /// The entity that paid the rent fees for the creation of this PDA.
    /// This will be referenced during the refund of the same upon closing this PDA.
    rent_sponsor: Pubkey,
    /// The number of slots after which (non-instant) refunds are allowed.
    /// This is stored so that it can later be verified through events.
    timelock: u64,
}

#[derive(Accounts)]
// The parameters must have the exact name and order as specified in the underlying function
// to avoid "seed constraint violation" errors.
// Refer: https://www.anchor-lang.com/docs/references/account-constraints#instruction-attribute
#[instruction(redeemer: Pubkey, refundee: Pubkey, secret_hash: [u8; 32], swap_amount: u64, timelock: u64)]
pub struct Initiate<'info> {
    /// A PDA that maintains the on-chain state of the atomic swap throughout its lifecycle.
    /// It also serves as the "vault" for this swap, by escrowing the SOL involved in this swap.
    /// The choice of seeds is to make the already expensive possibility of frontrunning, more expensive.
    /// This PDA will be deleted upon completion of the swap and the resulting rent would be returned
    /// to the rent sponsor.
    #[account(
        init,
        payer = rent_sponsor,
        seeds = [
            redeemer.as_ref(),
            refundee.as_ref(),
            &secret_hash,
            &swap_amount.to_le_bytes(),
            &timelock.to_le_bytes(),
        ],
        bump,
        space = ANCHOR_DISCRIMINATOR + SwapAccount::INIT_SPACE,
    )]
    pub swap_account: Account<'info, SwapAccount>,

    /// The party that deposits the funds to be involved in the atomic swap.
    /// They must sign this transaction.
    #[account(mut)]
    pub funder: Signer<'info>,

    /// Any entity that pays the PDA rent.
    /// Upon completion of the swap, the PDA rent refund resulting from the
    /// deletion of `swap_account` will be refunded to this address.
    #[account(mut)]
    pub rent_sponsor: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    /// The PDA holding the state information of the atomic swap.
    #[account(
        mut,
        seeds = [
            swap_account.redeemer.as_ref(),
            swap_account.refundee.key().as_ref(),
            &swap_account.secret_hash,
            &swap_account.swap_amount.to_le_bytes(),
            &swap_account.timelock.to_le_bytes(),
        ],
        bump = swap_account.bump,
        close = rent_sponsor,
    )]
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: Verifying the redeemer
    #[account(mut, address = swap_account.redeemer @ SwapError::InvalidRedeemer)]
    pub redeemer: AccountInfo<'info>,

    /// CHECK: Rent sponsor's address for refunding PDA rent
    #[account(mut, address = swap_account.rent_sponsor @ SwapError::InvalidRentSponsor)]
    pub rent_sponsor: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct Refund<'info> {
    /// The PDA holding the state information of the atomic swap.
    #[account(
        mut,
        seeds = [
            swap_account.redeemer.as_ref(),
            swap_account.refundee.key().as_ref(),
            &swap_account.secret_hash,
            &swap_account.swap_amount.to_le_bytes(),
            &swap_account.timelock.to_le_bytes(),
        ],
        bump = swap_account.bump,
        close = rent_sponsor,
    )]
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: The refundee of the swap.
    #[account(mut, address = swap_account.refundee @ SwapError::InvalidRefundee)]
    pub refundee: AccountInfo<'info>,

    /// CHECK: Rent sponsor's address for refunding PDA rent
    #[account(mut, address = swap_account.rent_sponsor @ SwapError::InvalidRentSponsor)]
    pub rent_sponsor: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct InstantRefund<'info> {
    /// The PDA holding the state information of the atomic swap.
    #[account(
        mut,
        seeds = [
            swap_account.redeemer.as_ref(),
            swap_account.refundee.key().as_ref(),
            &swap_account.secret_hash,
            &swap_account.swap_amount.to_le_bytes(),
            &swap_account.timelock.to_le_bytes(),
        ],
        bump = swap_account.bump,
        close = rent_sponsor,
    )]
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: The refundee of the swap.
    #[account(mut, address = swap_account.refundee @ SwapError::InvalidRefundee)]
    pub refundee: AccountInfo<'info>,

    /// CHECK: The redeemer of the swap. They must sign this transaction.
    #[account(address = swap_account.redeemer @ SwapError::InvalidRedeemer)]
    pub redeemer: Signer<'info>,

    /// CHECK: Rent sponsor's address for PDA rent refund
    #[account(mut, address = swap_account.rent_sponsor @ SwapError::InvalidRentSponsor)]
    pub rent_sponsor: AccountInfo<'info>,
}

/// Represents the initiated state of the swap where the funder has deposited funds into the vault
#[event]
pub struct Initiated {
    pub redeemer: Pubkey,
    pub refundee: Pubkey,
    pub secret_hash: [u8; 32],
    /// The quantity of native SOL transferred through this atomic swap in base units (aka lamports).  
    /// E.g: A quantity of 1 SOL will be represented as 1,000,000,000.
    pub swap_amount: u64,
    /// `timelock` represents the number of slots (1 slot = 400ms) after which
    /// (non-instant) refunds are allowed
    pub timelock: u64,
    /// Information regarding the destination chain in the atomic swap.
    pub destination_data: Option<Vec<u8>>,
    /// The party that deposited the funds for the atomic swap.
    pub funder: Pubkey,
}
/// Represents the redeemed state of the swap, where the redeemer has withdrawn funds from the vault.
/// Note that the secret is emitted here, in place of the secret hash.
#[event]
pub struct Redeemed {
    pub redeemer: Pubkey,
    pub refundee: Pubkey,
    pub secret: [u8; 32],
    pub swap_amount: u64,
    pub timelock: u64,
}
/// Represents the refund state of the swap, where the funds have been refunded past expiry
#[event]
pub struct Refunded {
    pub redeemer: Pubkey,
    pub refundee: Pubkey,
    pub secret_hash: [u8; 32],
    pub swap_amount: u64,
    pub timelock: u64,
}
/// Represents the instant refund state of the swap, where the funds have been refunded
/// with the redeemer's consent
#[event]
pub struct InstantRefunded {
    pub redeemer: Pubkey,
    pub refundee: Pubkey,
    pub secret_hash: [u8; 32],
    pub swap_amount: u64,
    pub timelock: u64,
}

#[error_code]
pub enum SwapError {
    #[msg("The provided refundee is incorrect")]
    InvalidRefundee,

    #[msg("The provided redeemer is not the original redeemer of this swap")]
    InvalidRedeemer,

    #[msg("The provided secret does not correspond to the secret hash of this swap")]
    InvalidSecret,

    #[msg("The provided rent sponsor is incorrect")]
    InvalidRentSponsor,

    #[msg("Attempt to refund before timelock expiry")]
    RefundBeforeExpiry,
}
