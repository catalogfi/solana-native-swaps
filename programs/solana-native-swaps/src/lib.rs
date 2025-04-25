use anchor_lang::{prelude::*, solana_program::hash, system_program};

declare_id!("6eksgdCnSjUaGQWZ6iYvauv1qzvYPF33RTGTM1ZuyENx");

#[program]
pub mod solana_native_swaps {
    use super::*;

    pub fn initiate(
        ctx: Context<Initiate>,
        amount_lamports: u64,
        expires_in_slots: u64,
        redeemer: Pubkey,
        secret_hash: [u8; 32],
    ) -> Result<()> {
        *ctx.accounts.swap_account = SwapAccount {
            amount_lamports,
            expiry_slot: Clock::get()?.slot + expires_in_slots,
            initiator: ctx.accounts.initiator.key(),
            redeemer,
            secret_hash,
        };

        let transfer_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.initiator.to_account_info(),
                to: ctx.accounts.swap_account.to_account_info(),
            },
        );
        system_program::transfer(transfer_context, amount_lamports)?;
        emit!(Initiated {
            amount_lamports,
            expires_in_slots,
            initiator: ctx.accounts.initiator.key(),
            redeemer,
            secret_hash,
        });

        Ok(())
    }

    pub fn redeem(ctx: Context<Redeem>, secret: [u8; 32]) -> Result<()> {
        let secret_hash = ctx.accounts.swap_account.secret_hash;
        require!(
            hash::hash(&secret).to_bytes() == secret_hash,
            SwapError::InvalidSecret
        );
        // Just emitting the secret should be enough, as the PDA is derived from secret hash,
        // and at any given time, there can only be one PDA.
        // Thus a malicious user cannot create another swap with the same secret hash,
        // till this swap is finished.
        emit!(Redeemed { secret });
        // All SOL in the swap account incl. rent fees will be transferred to the redeemer
        // by the 'close' attribute in the Redeem struct
        Ok(())
    }

    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        let secret_hash = ctx.accounts.swap_account.secret_hash;
        let expiry_slot = ctx.accounts.swap_account.expiry_slot;
        let current_slot = Clock::get()?.slot;
        require!(current_slot > expiry_slot, SwapError::RefundBeforeExpiry);
        emit!(Refunded { secret_hash });
        // All SOL in the swap account incl. rent fees will be transferred to the refundee
        // by the 'close' attribute in the Refund struct
        Ok(())
    }

    pub fn instant_refund(ctx: Context<InstantRefund>) -> Result<()> {
        emit!(InstantRefunded {
            secret_hash: ctx.accounts.swap_account.secret_hash
        });
        // All SOL in the swap account incl. rent fees will be transferred to the refundee
        // by the 'close' attribute in the RefundInstant struct
        Ok(())
    }
}

#[account]
pub struct SwapAccount {
    amount_lamports: u64,
    expiry_slot: u64,
    initiator: Pubkey,
    redeemer: Pubkey,
    secret_hash: [u8; 32],
}

#[derive(Accounts)]
// https://www.anchor-lang.com/docs/account-constraints#instruction-attribute
#[instruction(amount: u64, expires_in_slots: u64, redeemer: Pubkey, secret_hash: [u8; 32])]
pub struct Initiate<'info> {
    #[account(
        init,
        payer = initiator,
        seeds = [b"swap_account".as_ref(), &secret_hash],
        bump,
        space = 8 + std::mem::size_of::<SwapAccount>(),
    )]
    pub swap_account: Account<'info, SwapAccount>,

    /// Initiator must sign this transaction.
    #[account(mut)]
    pub initiator: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    #[account(mut, close = redeemer)]
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: `redeemer` here and redeemer provided during initiate() must be equal
    #[account(mut, address = swap_account.redeemer @ SwapError::InvalidRedeemer)]
    pub redeemer: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct Refund<'info> {
    #[account(mut, close = refundee)]
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: `refundee` here and initiator provided during initiate() must be equal
    #[account(mut, address = swap_account.initiator @ SwapError::InvalidRefundee)]
    pub refundee: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct InstantRefund<'info> {
    #[account(mut, close = initiator)]
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: `initiator` here and initiator provided during initiate() must be equal
    #[account(mut, address = swap_account.initiator @ SwapError::InvalidRefundee)]
    pub initiator: AccountInfo<'info>,

    /// CHECK: `redeemer` here and redeemer provided during initiate() must be equal.
    /// Redeemer must sign this transaction.
    #[account(address = swap_account.redeemer @ SwapError::InvalidRedeemer)]
    pub redeemer: Signer<'info>,
}

#[event]
pub struct Initiated {
    pub amount_lamports: u64,
    pub expires_in_slots: u64,
    pub initiator: Pubkey,
    pub redeemer: Pubkey,
    pub secret_hash: [u8; 32],
}
#[event]
pub struct Redeemed {
    pub secret: [u8; 32],
}
#[event]
pub struct Refunded {
    pub secret_hash: [u8; 32],
}
#[event]
pub struct InstantRefunded {
    pub secret_hash: [u8; 32],
}

#[error_code]
pub enum SwapError {
    #[msg("The provided redeemer is not the intended recipient of the swap amount")]
    InvalidRedeemer,

    #[msg("The provided refundee is not the initiator of the given swap account")]
    InvalidRefundee,

    #[msg("The provided secret does not correspond to the secret hash in the PDA")]
    InvalidSecret,

    #[msg("Attempt to perform a refund before expiry time")]
    RefundBeforeExpiry,
}
