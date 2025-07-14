use anchor_lang::{prelude::*, solana_program::hash, system_program};

declare_id!("6eksgdCnSjUaGQWZ6iYvauv1qzvYPF33RTGTM1ZuyENx");

/// The size of Anchor's internal discriminator in a PDA's memory
const ANCHOR_DISCRIMINATOR: usize = 8;

#[program]
pub mod solana_native_swaps {
    use super::*;

    /// Initiates the atomic swap. Funds are transferred from the initiator to the token vault.
    /// As such, the initiator's signature is required for this instruction.
    /// `swap_amount` represents the quantity of native SOL to be transferred
    /// through this atomic swap in base units (aka lamports).  
    /// E.g: A quantity of 1 SOL must be provided as 1,000,000,000.
    /// `expires_in_slots` represents the number of slots (1 slot = 400ms) after
    /// which (non-instant) refunds are allowed.
    pub fn initiate(
        ctx: Context<Initiate>,
        expires_in_slots: u64,
        redeemer: Pubkey,
        secret_hash: [u8; 32],
        swap_amount: u64,
        destination_data: Option<Vec<u8>>,
    ) -> Result<()> {
        let transfer_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.initiator.to_account_info(),
                to: ctx.accounts.swap_account.to_account_info(),
            },
        );
        system_program::transfer(transfer_context, swap_amount)?;

        *ctx.accounts.swap_account = SwapAccount {
            expiry_slot: Clock::get()?.slot + expires_in_slots,
            bump: ctx.bumps.swap_account,
            sponsor: ctx.accounts.sponsor.key(),
            expires_in_slots,
            initiator: ctx.accounts.initiator.key(),
            redeemer,
            secret_hash,
            swap_amount,
        };

        emit!(Initiated {
            expires_in_slots,
            initiator: ctx.accounts.initiator.key(),
            redeemer,
            secret_hash,
            swap_amount,
            destination_data,
        });

        Ok(())
    }

    /// Funds are transferred to the redeemer. This instruction does not require any signatures.
    pub fn redeem(ctx: Context<Redeem>, secret: [u8; 32]) -> Result<()> {
        let SwapAccount {
            expires_in_slots,
            initiator,
            redeemer,
            secret_hash,
            swap_amount,
            ..
        } = *ctx.accounts.swap_account;

        require!(
            hash::hash(&secret).to_bytes() == secret_hash,
            SwapError::InvalidSecret
        );

        ctx.accounts.swap_account.sub_lamports(swap_amount)?;
        ctx.accounts.redeemer.add_lamports(swap_amount)?;

        emit!(Redeemed {
            expires_in_slots,
            initiator,
            redeemer,
            secret,
            swap_amount,
        });

        Ok(())
    }

    /// Funds are returned to the initiator, given that no redeems have occured
    /// and the expiry slot has been reached.
    /// This instruction does not require any signatures.
    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        let SwapAccount {
            expiry_slot,
            expires_in_slots,
            initiator,
            redeemer,
            secret_hash,
            swap_amount,
            ..
        } = *ctx.accounts.swap_account;

        let current_slot = Clock::get()?.slot;
        require!(current_slot > expiry_slot, SwapError::RefundBeforeExpiry);

        ctx.accounts.swap_account.sub_lamports(swap_amount)?;
        ctx.accounts.initiator.add_lamports(swap_amount)?;

        emit!(Refunded {
            expires_in_slots,
            initiator,
            redeemer,
            secret_hash,
            swap_amount,
        });

        Ok(())
    }

    /// Funds are returned to the initiator, with the redeemer's consent.
    /// As such, the redeemer's signature is required for this instruction.
    /// This allows for refunds before the expiry slot.
    pub fn instant_refund(ctx: Context<InstantRefund>) -> Result<()> {
        let SwapAccount {
            expires_in_slots,
            initiator,
            redeemer,
            secret_hash,
            swap_amount,
            ..
        } = *ctx.accounts.swap_account;

        ctx.accounts.swap_account.sub_lamports(swap_amount)?;
        ctx.accounts.initiator.add_lamports(swap_amount)?;

        emit!(InstantRefunded {
            expires_in_slots,
            initiator,
            redeemer,
            secret_hash,
            swap_amount,
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

    /// The number of slots after which (non-instant) refunds are allowed.
    /// This is stored so that it can later be verified through events.
    expires_in_slots: u64,
    /// The initiator of the atomic swap
    initiator: Pubkey,
    /// The redeemer of the atomic swap
    redeemer: Pubkey,
    /// The secret hash associated with the atomic swap
    secret_hash: [u8; 32],
    /// The quantity of native SOL to be transferred through this atomic swap in base units (aka lamports)
    swap_amount: u64,
    /// The entity that paid the rent fees for the creation of this PDA.
    /// This will be referenced during the refund of the same upon closing this PDA.
    sponsor: Pubkey,
}

#[derive(Accounts)]
// The parameters must have the exact name and order as specified in the underlying function
// to avoid "seed constraint violation" errors.
// Refer: https://www.anchor-lang.com/docs/references/account-constraints#instruction-attribute
#[instruction(expires_in_slots: u64, redeemer: Pubkey, secret_hash: [u8; 32], swap_amount: u64)]
pub struct Initiate<'info> {
    /// A PDA that maintains the on-chain state of the atomic swap throughout its lifecycle.
    /// It also serves as the "vault" for this swap, by escrowing the SOL involved in this swap.
    /// The choice of seeds is to make the already expensive possibility of frontrunning, more expensive.
    /// This PDA will be deleted upon completion of the swap.
    #[account(
        init,
        payer = sponsor,
        seeds = [
            &expires_in_slots.to_le_bytes(),
            initiator.key().as_ref(),
            redeemer.as_ref(),
            &secret_hash,
            &swap_amount.to_le_bytes(),
        ],
        bump,
        space = ANCHOR_DISCRIMINATOR + SwapAccount::INIT_SPACE,
    )]
    pub swap_account: Account<'info, SwapAccount>,

    /// The initiator of the atomic swap. They must sign this transaction.
    #[account(mut)]
    pub initiator: Signer<'info>,

    /// Any entity that pays the PDA rent.
    /// Upon completion of the swap, the PDA rent refund resulting from the
    /// deletion of `swap_account` will be refunded to this address.
    #[account(mut)]
    pub sponsor: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    /// The PDA holding the state information of the atomic swap.
    /// Will be closed upon successful execution and the resulting rent
    /// will be transferred to the initiator.
    #[account(
        mut,
        seeds = [
            &swap_account.expires_in_slots.to_le_bytes(),
            swap_account.initiator.key().as_ref(),
            swap_account.redeemer.as_ref(),
            &swap_account.secret_hash,
            &swap_account.swap_amount.to_le_bytes(),
        ],
        bump = swap_account.bump,
        close = sponsor,
    )]
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: Verifying the redeemer
    #[account(mut, address = swap_account.redeemer @ SwapError::InvalidRedeemer)]
    pub redeemer: AccountInfo<'info>,

    /// CHECK: Sponsor's address for refunding PDA rent
    #[account(mut, address = swap_account.sponsor @ SwapError::InvalidSponsor)]
    pub sponsor: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct Refund<'info> {
    /// The PDA holding the state information of the atomic swap.
    /// Will be closed upon successful execution and the resulting rent
    /// will be transferred to the initiator.
    #[account(
        mut,
        seeds = [
            &swap_account.expires_in_slots.to_le_bytes(),
            swap_account.initiator.key().as_ref(),
            swap_account.redeemer.as_ref(),
            &swap_account.secret_hash,
            &swap_account.swap_amount.to_le_bytes(),
        ],
        bump = swap_account.bump,
        close = sponsor,
    )]
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: The initiator of the swap.
    #[account(mut, address = swap_account.initiator @ SwapError::InvalidInitiator)]
    pub initiator: AccountInfo<'info>,

    /// CHECK: Sponsor's address for refunding PDA rent
    #[account(mut, address = swap_account.sponsor @ SwapError::InvalidSponsor)]
    pub sponsor: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct InstantRefund<'info> {
    /// The PDA holding the state information of the atomic swap.
    /// Will be closed upon successful execution and the resulting rent
    /// will be transferred to the initiator.
    #[account(
        mut,
        seeds = [
            &swap_account.expires_in_slots.to_le_bytes(),
            swap_account.initiator.key().as_ref(),
            swap_account.redeemer.as_ref(),
            &swap_account.secret_hash,
            &swap_account.swap_amount.to_le_bytes(),
        ],
        bump = swap_account.bump,
        close = sponsor,
    )]
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: The initiator of the swap.
    #[account(mut, address = swap_account.initiator @ SwapError::InvalidInitiator)]
    pub initiator: AccountInfo<'info>,

    /// CHECK: The redeemer of the swap. They must sign this transaction.
    #[account(address = swap_account.redeemer @ SwapError::InvalidRedeemer)]
    pub redeemer: Signer<'info>,

    /// CHECK: Sponsor's address for PDA rent refund
    #[account(mut, address = swap_account.sponsor @ SwapError::InvalidSponsor)]
    pub sponsor: AccountInfo<'info>,
}

/// Represents the initiated state of the swap where the initiator has deposited funds into the vault
#[event]
pub struct Initiated {
    /// `expires_in_slots` represents the number of slots (1 slot = 400ms) after which
    /// (non-instant) refunds are allowed
    pub expires_in_slots: u64,
    pub initiator: Pubkey,
    pub redeemer: Pubkey,
    pub secret_hash: [u8; 32],
    /// The quantity of native SOL transferred through this atomic swap in base units (aka lamports).  
    /// E.g: A quantity of 1 SOL will be represented as 1,000,000,000.
    pub swap_amount: u64,
    /// Information regarding the destination chain in the atomic swap.
    pub destination_data: Option<Vec<u8>>,
}
/// Represents the redeemed state of the swap, where the redeemer has withdrawn funds from the vault.
/// Note that the secret is emitted here, in place of the secret hash.
#[event]
pub struct Redeemed {
    pub expires_in_slots: u64,
    pub initiator: Pubkey,
    pub redeemer: Pubkey,
    pub secret: [u8; 32],
    pub swap_amount: u64,
}
/// Represents the refund state of the swap, where the initiator has withdrawn funds from the vault past expiry
#[event]
pub struct Refunded {
    pub expires_in_slots: u64,
    pub initiator: Pubkey,
    pub redeemer: Pubkey,
    pub secret_hash: [u8; 32],
    pub swap_amount: u64,
}
/// Represents the instant refund state of the swap, where the initiator has withdrawn funds the vault
/// with the redeemer's consent
#[event]
pub struct InstantRefunded {
    pub expires_in_slots: u64,
    pub initiator: Pubkey,
    pub redeemer: Pubkey,
    pub secret_hash: [u8; 32],
    pub swap_amount: u64,
}

#[error_code]
pub enum SwapError {
    #[msg("The provided initiator is not the original initiator of this swap")]
    InvalidInitiator,

    #[msg("The provided redeemer is not the original redeemer of this swap")]
    InvalidRedeemer,

    #[msg("The provided secret does not correspond to the secret hash of this swap")]
    InvalidSecret,

    #[msg("The provided sponsor is not the original sponsor of this swap")]
    InvalidSponsor,

    #[msg("Attempt to perform a refund before expiry time")]
    RefundBeforeExpiry,
}
