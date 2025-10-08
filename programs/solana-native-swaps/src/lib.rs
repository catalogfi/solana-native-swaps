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

    // ================ UDA Logic ================

    /// Create a native SOL Universal Data Agreement with mandatory registry validation
    /// 
    /// Creates a UDA for native SOL transfers with destination data for cross-chain routing.
    /// The HTLC program must be registered in the registry for the operation to succeed.
    /// 
    /// # Arguments
    /// * `ctx` - Context containing all required accounts
    /// * `amount` - Amount of lamports to lock in the UDA
    /// * `refund_address` - Address to receive funds if the UDA expires
    /// * `redeemer` - Address authorized to redeem the UDA
    /// * `timelock` - Slot number when the UDA expires
    /// * `secret_hash` - Hash of the secret required for redemption
    /// * `destination_data` - Cross-chain routing data
    /// 
    /// # Returns
    /// * `Result<()>` - Success or error
    /// 
    /// # Errors
    /// * `ZeroAmount` - If amount is zero
    /// * `SameAddress` - If refund_address equals redeemer
    /// * `InvalidAddress` - If any address is the default pubkey
    /// * `InvalidHTLCProgram` - If HTLC program is not registered
    /// * `InvalidTimelock` - If timelock is zero
    /// * `InvalidSecretHash` - If secret hash is all zeros or destination hash mismatch
    pub fn create_uda_native(
        ctx: Context<CreateNativeUDA>,
        amount: u64,
        refund_address: Pubkey,
        redeemer: Pubkey,
        timelock: u64,
        secret_hash: [u8; 32],
        destination_data: Vec<u8>,
        destination_hash: [u8; 32],
    ) -> Result<()> {
        require!(amount > 0, UDAError::ZeroAmount);
        require!(refund_address != redeemer, UDAError::SameAddress);
        require!(refund_address != Pubkey::default(), UDAError::InvalidAddress);
        require!(redeemer != Pubkey::default(), UDAError::InvalidAddress);
        require!(timelock > 0, UDAError::InvalidTimelock);
        require!(secret_hash != [0u8; 32], UDAError::InvalidSecretHash);

        let computed_hash = hash(&destination_data).to_bytes();
        require!(destination_hash == computed_hash, UDAError::InvalidSecretHash);

        let clock = Clock::get()?;
        let current_slot = clock.slot;

        let uda = &mut ctx.accounts.uda;
        require!(uda.key() != redeemer, UDAError::SameAddress);

        uda.refund_address = refund_address;
        uda.redeemer = redeemer;
        uda.timelock = timelock;
        uda.secret_hash = secret_hash;
        uda.amount = amount;
        uda.created_at = current_slot;
        uda.rent_sponsor = ctx.accounts.payer.key();
        uda.destination_data = destination_data.clone();
        uda.destination_hash = destination_hash;

        transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                SystemTransfer {
                    from: ctx.accounts.payer.to_account_info(),
                    to: ctx.accounts.uda.to_account_info(),
                },
            ),
            amount,
        )?;

        // Emit event
        emit!(UDACreated {
            uda_address: ctx.accounts.uda.key(),
            refund_address,
            amount,
            timelock,
        });

        Ok(())
    }

    /// Initiate Hash Time Lock Contract for a native SOL UDA
    /// 
    /// Creates and executes an HTLC instruction on the registered HTLC program
    /// for native SOL transfers. After initiation, transfers any excess SOL to 
    /// the refund address and closes the UDA account, returning rent to the sponsor.
    /// 
    /// # Arguments
    /// * `ctx` - Context containing UDA, HTLC program, and cleanup accounts
    /// 
    /// # Returns
    /// * `Result<()>` - Success or error
    /// 
    /// # Errors
    /// * `InvalidState` - If UDA is not in Created state
    /// * `InvalidTimelock` - If timelock is zero
    /// * `InvalidHTLCProgram` - If HTLC program doesn't match UDA registration
    /// * `InsufficientFunds` - If UDA doesn't have enough SOL for the operation
    pub fn initiate_htlc_native(ctx: Context<InitiateNativeHTLC>) -> Result<()> {
        let uda = &mut ctx.accounts.uda;

        let rent_exempt = Rent::get()?.minimum_balance(uda.to_account_info().data_len());
        require!(
            uda.to_account_info().lamports() >= uda.amount.checked_add(rent_exempt).ok_or(UDAError::InsufficientFunds)?,
            UDAError::InsufficientFunds
        );

        let current_slot = Clock::get()?.slot;
        let timelock_slots = uda.timelock.checked_sub(current_slot).ok_or(UDAError::Expired)?; // @audit if you revert here, user cannot recover funds

        let refund_seed = uda.refund_address.key();
        let redeemer_seed = uda.redeemer.key();

        let signer_seeds: &[&[&[u8]]] = &[&[
            b"native_uda",
            refund_seed.as_ref(),
            redeemer_seed.as_ref(),
            &uda.secret_hash,
            &uda.amount.to_le_bytes(),
            &uda.timelock.to_le_bytes(),
            &uda.destination_hash,
        ]];

        let instruction = anchor_lang::solana_program::instruction::Instruction {
            program_id: uda.htlc_program,
            accounts: vec![
                anchor_lang::solana_program::instruction::AccountMeta::new(
                    ctx.accounts.swap_account.key(),
                    false,
                ),
                anchor_lang::solana_program::instruction::AccountMeta::new(
                    uda.to_account_info().key(),
                    true,
                ),
                anchor_lang::solana_program::instruction::AccountMeta::new_readonly(
                    ctx.accounts.system_program.key(),
                    false,
                ),
            ],
            data: {
                let destination_data = &uda.destination_data;
                let mut data = Vec::with_capacity(8 + 32 + 32 + 32 + 8 + 8 + 1 + 4 + destination_data.len());
                
                let discriminator = anchor_lang::solana_program::hash::hash(b"global:initiate")
                    .to_bytes()[..8].to_vec();
                data.extend_from_slice(&discriminator);
                
                data.extend_from_slice(&uda.redeemer.to_bytes());
                data.extend_from_slice(&uda.refund_address.to_bytes());
                data.extend_from_slice(&uda.secret_hash);
                data.extend_from_slice(&uda.amount.to_le_bytes());
                data.extend_from_slice(&timelock_slots.to_le_bytes());
                
                // Add destination_data length and content
                data.extend_from_slice(&(destination_data.len() as u32).to_le_bytes());
                data.extend_from_slice(destination_data);
                
                data
            },
        };

        anchor_lang::solana_program::program::invoke_signed(
            &instruction,
            &[
                ctx.accounts.swap_account.to_account_info(),
                uda.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            signer_seeds,
        )?;

        // Calculate excess SOL after HTLC initiation and rent exemption
        let current_balance = uda.to_account_info().lamports();
        let rent_exempt = Rent::get()?.minimum_balance(uda.to_account_info().data_len());
        
        // Transfer any excess SOL to refund address before closing
        if current_balance > rent_exempt {
            let excess_amount = current_balance.checked_sub(rent_exempt).ok_or(UDAError::InsufficientFunds)?;
            
            if excess_amount > 0 {
                **uda.to_account_info().try_borrow_mut_lamports()? = 
                    current_balance.checked_sub(excess_amount).ok_or(UDAError::InsufficientFunds)?;
                **ctx.accounts.refund_address.to_account_info().try_borrow_mut_lamports()? = 
                    ctx.accounts.refund_address.to_account_info().lamports().checked_add(excess_amount).ok_or(UDAError::InsufficientFunds)?;
            }
        }

        emit!(HTLCInitiated {
            uda_address: uda.key(),
            swap_amount: uda.amount,
            timelock: uda.timelock,
        });

        Ok(())
    }
}

#[account]
#[derive(InitSpace)]
pub struct NativeUDA {
    pub refund_address: Pubkey,
    pub redeemer: Pubkey,
    pub timelock: u64,
    pub secret_hash: [u8; 32],
    pub amount: u64,
    pub created_at: u64,        // Slot when UDA was created (0 = not created, >0 = created)
    pub rent_sponsor: Pubkey, // Who paid for UDA creation (gets rent back)
    #[max_len(10240)]  // Large limit - user pays for storage
    pub destination_data: Vec<u8>, // Destination data for cross-chain/routing purposes
    pub destination_hash: [u8; 32],  // SHA256 hash of destination_data for PDA seeds
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
#[instruction(
    refund_address: Pubkey,
    redeemer: Pubkey,
    timelock: u64,
    secret_hash: [u8; 32],
    amount: u64,
    htlc_program: Pubkey,
    destination_data: Vec<u8>,
)]
pub struct CreateNativeUDA<'info> {
    // Mandatory registry account for validation - ensures only authorized HTLC programs can be used
    #[account(
        seeds = [b"htlc_registry"],
        bump,
    )]
    pub registry: Account<'info, HTLCRegistry>,
    
    #[account(
        init,
        payer = payer,
        seeds = [
            b"native_uda",
            refund_address.as_ref(),
            redeemer.as_ref(),
            &secret_hash,
            &amount.to_le_bytes(),
            &timelock.to_le_bytes(),
        ],
        bump,
        space = NativeUDA::INIT_SPACE,
    )]
    pub uda: Account<'info, NativeUDA>,

    #[account(mut)]
    pub payer: Signer<'info>,

    pub system_program: Program<'info, System>,
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

#[event]
pub struct UDACreated {
    pub uda_address: Pubkey,
    pub refund_address: Pubkey,
    pub amount: u64,
    pub timelock: u64,
}

#[event]
pub struct HTLCInitiated {
    pub uda_address: Pubkey,
    pub htlc_program: Pubkey,
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
