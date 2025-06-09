use anchor_lang::{prelude::*, solana_program::hash, system_program};

declare_id!("2bag6xpshpvPe7SJ9nSDLHpxqhEAoHPGpEkjNSv7gxoF");

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
#[instruction(amount_lamports: u64, expires_in_slots: u64, redeemer: Pubkey, secret_hash: [u8; 32])]
pub struct Initiate<'info> {
    #[account(
        init,
        payer = initiator,
        seeds = [b"swap_account", initiator.key().as_ref(), &secret_hash],
        bump,
        space = 8 + std::mem::size_of::<SwapAccount>(),
    )]
    pub swap_account: Account<'info, SwapAccount>,

    /// Initiator must sign this transaction
    #[account(mut)]
    pub initiator: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    #[account(mut, close = initiator)]
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: Verifying the initiator.  
    /// This is included here for the PDA rent refund using the `close` attribute above
    #[account(mut, address = swap_account.initiator @ SwapError::InvalidInitiator)]
    pub initiator: AccountInfo<'info>,

    /// CHECK: Verifying the redeemer
    #[account(mut, address = swap_account.redeemer @ SwapError::InvalidRedeemer)]
    pub redeemer: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct Refund<'info> {
    #[account(mut, close = initiator)]
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: Verifying the initiator  
    #[account(mut, address = swap_account.initiator @ SwapError::InvalidInitiator)]
    pub initiator: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct InstantRefund<'info> {
    #[account(mut, close = initiator)]
    pub swap_account: Account<'info, SwapAccount>,

    /// CHECK: Verifying the initiator
    #[account(mut, address = swap_account.initiator @ SwapError::InvalidInitiator)]
    pub initiator: AccountInfo<'info>,

    /// CHECK: Verifying the redeemer.  
    /// Redeemer must sign this transaction
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
    pub initiator: Pubkey,
    pub secret: [u8; 32],
}
#[event]
pub struct Refunded {
    pub initiator: Pubkey,
    pub secret_hash: [u8; 32],
}
#[event]
pub struct InstantRefunded {
    pub initiator: Pubkey,
    pub secret_hash: [u8; 32],
}

#[error_code]
pub enum SwapError {
    #[msg("The provided initiator is not the original initiator of this swap account")]
    InvalidInitiator,

    #[msg("The provided redeemer is not the original redeemer of this swap amount")]
    InvalidRedeemer,

    #[msg("The provided secret does not correspond to the secret hash in the swap account")]
    InvalidSecret,

    #[msg("Attempt to perform a refund before expiry time")]
    RefundBeforeExpiry,
}
