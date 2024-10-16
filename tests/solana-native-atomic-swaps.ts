import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { SolanaNativeAtomicSwaps } from "../target/types/solana_native_atomic_swaps";

import * as crypto from 'crypto';
import { expect } from "chai";

// Configure the client to use the local cluster.
anchor.setProvider(anchor.AnchorProvider.env());
const connection = anchor.getProvider().connection;
const program = anchor.workspace.SolanaNativeAtomicSwaps as Program<SolanaNativeAtomicSwaps>;
const LAMPORTS_PER_SOL = anchor.web3.LAMPORTS_PER_SOL;
const MILLIS_PER_SLOT = 400;

describe("Testing one way swap between Alice and Bob", () => {
    const swapAmount = new anchor.BN(0.1 * LAMPORTS_PER_SOL);
    const swapExpiresIn = new anchor.BN(1000 / MILLIS_PER_SLOT); // 1 second

    // Alice is the initiator here
    // Alice's stuff
    const alice = new anchor.web3.Keypair();
    const alicePubkey = alice.publicKey;
    const secret = Buffer.from(Array(32).fill(0));
    const secretHash = [...(crypto.createHash('sha256').update(secret).digest())];

    // Bob's stuff
    const bob = new anchor.web3.Keypair();
    const bobPubkey = bob.publicKey;

    // PDA
    const pdaSeeds = [Buffer.from("swap_account"), alicePubkey.toBuffer(), Buffer.from(secretHash)];
    const [swapAccount,] = anchor.web3.PublicKey.findProgramAddressSync(pdaSeeds, program.programId);
    const size = program.account.swapAccount.size;
    let rentAmount: number;

    console.log(`Alice: ${alicePubkey}\nBob: ${bobPubkey}\nSwap Account (PDA): ${swapAccount}`);

    const aliceInitiate = () => new Promise<void>(async resolve => {
        await program.methods.initiate(bobPubkey, secretHash, swapAmount, swapExpiresIn)
            .accounts({
                initiator: alicePubkey,
            }).signers([alice]).rpc()
            .then(async signature => {
                console.log("Alice initiated with Signature:", signature);
                await connection.confirmTransaction({signature, ...(await connection.getLatestBlockhash())});
            });
            resolve();
        }
    );

    before(async () => {
        // Fund alice's wallet with 1 SOL
        await connection.requestAirdrop(alicePubkey, anchor.web3.LAMPORTS_PER_SOL)
        .then(async signature =>
            // For some program sizes, larger rentAmount (e.g. six bytes more worth) is taken somehow
            await connection.confirmTransaction({signature, ...(await connection.getLatestBlockhash())}));
        rentAmount = await connection.getMinimumBalanceForRentExemption(size);
    });

    it("Test initiation", async () => {
        await aliceInitiate();
        const pdaBalance = await connection.getBalance(swapAccount);
        expect(pdaBalance - rentAmount).to.equal(swapAmount.toNumber());

    });

    it("Test redeem", async () => {
        // The previous test has already initiated the swap
        await program.methods.redeem([...secret])
        .accounts({
            swapAccount,
            redeemer: bobPubkey,
        }).rpc()
        .then(async signature => {
            console.log("Bob redeemed with Signature:", signature);
            await connection.confirmTransaction({signature, ...(await connection.getLatestBlockhash())});
        });

        const pdaBalance = await connection.getBalance(swapAccount);
        expect(pdaBalance).to.equal(0);
    });

    it("Test refund", async () => {
        await aliceInitiate();  // Re-initiating for the sake of testcase

        console.log("Awaiting timelock for refund");
        await new Promise(r => setTimeout(r, swapExpiresIn.toNumber() * MILLIS_PER_SLOT));
        await program.methods.refund()
        .accounts({
            swapAccount,
            refundee: alicePubkey,
        }).rpc()
        .then(async signature => {
            console.log("Alice refunded with Signature:", signature);
            await connection.confirmTransaction({signature, ...(await connection.getLatestBlockhash())});
        });

        const pdaBalance = await connection.getBalance(swapAccount);
        expect(pdaBalance).to.equal(0);
    });

    it("Test instant refund", async () => {
        await aliceInitiate();  // Re-initiating for the sake of testcase
        await program.methods.instantRefund()
        .accounts({
            swapAccount,
            initiator: alicePubkey,
            redeemer: bobPubkey,
        }).signers([alice, bob])
        .rpc()
        .then(async signature => {
            console.log("Alice instant-refunded with Signature:", signature);
            await connection.confirmTransaction({signature, ...(await connection.getLatestBlockhash())});
        });

        const pdaBalance = await connection.getBalance(swapAccount);
        expect(pdaBalance).to.equal(0);
    });
});
