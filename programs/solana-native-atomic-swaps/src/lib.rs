use anchor_lang::prelude::*;
use anchor_lang::system_program;
use solana_program::hash;

declare_id!("GmmfyjGJaSaH7ZgRp4BYEr7v9vydtd4NKg4yJnWdzRgj");

type Lamports = u64; // 1 SOL = 10^9 lamports
type Slots = u64; // 1 slot = 400ms

#[program]
pub mod solana_native_atomic_swaps {
    use super::*;

    pub fn initiate(
        ctx: Context<Initiate>,
        swap_id: [u8; 32],
        redeemer: Pubkey,
        secret_hash: [u8; 32],
        amount: Lamports,
        expires_in: Slots,
    ) -> Result<()> {
        *ctx.accounts.swap_account = SwapAccount {
            swap_id,
            redeemer,
            secret_hash,
            amount,
            initiator: ctx.accounts.initiator.key(),
            expiry_slot: Clock::get()?.slot + expires_in,
        };

        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.initiator.to_account_info(),
                to: ctx.accounts.swap_account.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, amount)?;
        emit!(Initiated { swap_id, secret_hash, amount });

        Ok(())
    }

    pub fn redeem(ctx: Context<Redeem>, secret: [u8; 32]) -> Result<()> {
        let SwapAccount { swap_id, secret_hash, .. } = *ctx.accounts.swap_account;
        require!(hash::hash(&secret).to_bytes() == secret_hash, SwapError::InvalidSecret);
        emit!(Redeemed { swap_id, secret });
        // All SOL in the swap account incl. rent fees will be transferred to the redeemer
        // by the 'close' attribute in the Redeem struct
        Ok(())
    }

    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        let SwapAccount { swap_id, expiry_slot, .. } = *ctx.accounts.swap_account;
        let current_slot = Clock::get()?.slot;
        require!(current_slot >= expiry_slot, SwapError::RefundBeforeExpiry);
        emit!(Refunded { swap_id });
        // All SOL in the swap account incl. rent fees will be transferred to the refundee
        // by the 'close' attribute in the Refund struct
        Ok(())
    }

    pub fn instant_refund(ctx: Context<InstantRefund>) -> Result<()> {
        emit!(InstantRefunded { swap_id: ctx.accounts.swap_account.swap_id });
        // All SOL in the swap account incl. rent fees will be transferred to the refundee
        // by the 'close' attribute in the RefundInstant struct
        Ok(())
    }
}

#[account]
pub struct SwapAccount {
    swap_id: [u8; 32],
    initiator: Pubkey,
    redeemer: Pubkey,
    secret_hash: [u8; 32],
    expiry_slot: u64,
    amount: Lamports,
}

#[derive(Accounts)]
// redeemer is included in #[instruction(...)] for reasons outlined in:
// https://www.anchor-lang.com/docs/account-constraints#instruction-attribute
#[instruction(swap_id: [u8; 32])]
pub struct Initiate<'info> {
    #[account(
        init,
        payer = initiator,
        seeds = [b"swap_account".as_ref(), &swap_id],
        bump,
        space = 8 + std::mem::size_of::<SwapAccount>(),
    )]
    pub swap_account: Account<'info, SwapAccount>,

    #[account(mut)]
    pub initiator: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    #[account(mut, close = redeemer)] // Closes and transfers all SOL to redeemer upon success
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: The public key of the redeemer
    #[account(mut, address = swap_account.redeemer @ SwapError::InvalidRedeemer)]
    pub redeemer: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct Refund<'info> {
    #[account(mut, close = refundee)] // Closes and transfers all SOL to refundee upon success
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: The public key of the initiator
    #[account(mut, address = swap_account.initiator @ SwapError::InvalidRefundee)]
    pub refundee: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct InstantRefund<'info> {
    #[account(mut, close = initiator)]
    // Closes and transfers all SOL to the initiator upon success
    pub swap_account: Account<'info, SwapAccount>,

    #[account(mut, address = swap_account.initiator @ SwapError::InvalidRefundee)]
    pub initiator: Signer<'info>,

    #[account(address = swap_account.redeemer @ SwapError::InvalidRedeemer)]
    pub redeemer: Signer<'info>,
}

#[event]
pub struct Initiated {
    swap_id: [u8; 32],
    secret_hash: [u8; 32],
    amount: u64,
}
#[event]
pub struct Redeemed {
    pub swap_id: [u8; 32],
    pub secret: [u8; 32],
}
#[event]
pub struct Refunded {
    pub swap_id: [u8; 32],
}
#[event]
pub struct InstantRefunded {
    pub swap_id: [u8; 32],
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