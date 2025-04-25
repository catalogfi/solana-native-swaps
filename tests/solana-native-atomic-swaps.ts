import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { SolanaNativeSwaps } from "../target/types/solana_native_swaps";

import * as crypto from 'crypto';
import { expect } from "chai";

// Configure the client to use the local cluster.
anchor.setProvider(anchor.AnchorProvider.env());
const connection = anchor.getProvider().connection;
const program = anchor.workspace.SolanaNativeSwaps as Program<SolanaNativeSwaps>;
const LAMPORTS_PER_SOL = anchor.web3.LAMPORTS_PER_SOL;
const MILLIS_PER_SLOT = 400;

describe("Testing one way swap between Alice and Bob", () => {
	const swapAmount = new anchor.BN(0.1 * LAMPORTS_PER_SOL);
	const expiresInSlots = new anchor.BN(800 / MILLIS_PER_SLOT); // 0.8 secs

	// Alice is the initiator here
	// Alice's stuff
	const alice = anchor.web3.Keypair.fromSeed(new Uint8Array(32).fill(0));
	const secret = Buffer.from(Array(32).fill(0));
	const secretHash = [...(crypto.createHash('sha256').update(secret).digest())];

	// Bob's stuff
	const bob = anchor.web3.Keypair.fromSeed(new Uint8Array(32).fill(1));

	// PDA
	const pdaSeeds = [Buffer.from("swap_account"), Buffer.from(secretHash)];
	const [swapAccount,] = anchor.web3.PublicKey.findProgramAddressSync(pdaSeeds, program.programId);
	const size = program.account.swapAccount.size;
	let rentAmount: number;

	console.log(`Alice: ${alice.publicKey}\nBob: ${bob.publicKey}\nSwap Account (PDA): ${swapAccount}`);

	const aliceInitiate = () => new Promise<void>(async resolve => {
		console.log("alice is initiating");
		await program.methods.initiate(swapAmount, expiresInSlots, bob.publicKey, secretHash)
			.accounts({
				initiator: alice.publicKey,
			}).signers([alice]).rpc()
			.then(async signature => {
				console.log("Alice initiated with Signature:", signature);
				await connection.confirmTransaction({signature, ...(await connection.getLatestBlockhash())});
			});
		resolve();
        }
	);

	before(async () => {
		console.log("performing airdrop of 1 SOL to alice");
		// Fund alice's wallet with 1 SOL
		await connection.requestAirdrop(alice.publicKey, anchor.web3.LAMPORTS_PER_SOL)
			.then(async signature =>
				// For some program sizes, larger rentAmount (e.g. six bytes more worth) is taken somehow
				await connection.confirmTransaction({signature, ...(await connection.getLatestBlockhash())}));
		console.log("airdrop successful");
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
				redeemer: bob.publicKey,
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
		await new Promise(r => setTimeout(r, (expiresInSlots.toNumber() + 1) * MILLIS_PER_SLOT));
		await program.methods.refund()
			.accounts({
				swapAccount,
				refundee: alice.publicKey,
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
				initiator: alice.publicKey,
				redeemer: bob.publicKey,
			}).signers([bob])
			.rpc()
			.then(async signature => {
				console.log("Alice instant-refunded with Signature:", signature);
				await connection.confirmTransaction({signature, ...(await connection.getLatestBlockhash())});
			});

		const pdaBalance = await connection.getBalance(swapAccount);
		expect(pdaBalance).to.equal(0);
	});
});
