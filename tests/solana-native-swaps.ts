import { web3, BN, workspace, getProvider, Program } from "@coral-xyz/anchor";
import crypto from "crypto";
import { expect } from "chai";

import { setTimeout } from "node:timers/promises";
import { SolanaNativeSwaps } from "../target/types/solana_native_swaps";

// Provider will use the private key at ~/.config/solana/id.json
const provider = getProvider();
const connection = provider.connection;
const program = workspace.SolanaNativeSwaps as Program<SolanaNativeSwaps>;

describe("Testing one way swap between Alice and Bob", () => {
  const swapAmount = new BN(0.1 * web3.LAMPORTS_PER_SOL);
  const expiresInSlots = new BN(5); // 2 secs (1 slot = 0.4 secs)
  // Alice is the initiator here
  const alice = web3.Keypair.fromSeed(crypto.randomBytes(32));
  const secret = crypto.randomBytes(32);
  const secretHash = crypto.createHash("sha256").update(secret).digest();

  // Bob is the redeemer here
  const bob = web3.Keypair.fromSeed(crypto.randomBytes(32));

  // SwapAccount PDA
  const pdaSeeds = [
    Buffer.from("swap_account"),
    alice.publicKey.toBuffer(),
    Buffer.from(secretHash),
  ];
  const [swapAccount] = web3.PublicKey.findProgramAddressSync(
    pdaSeeds,
    program.programId
  );
  let rentAmount: number;

  console.log({ alice: alice.publicKey, bob: bob.publicKey, swapAccount });

  const aliceInitiate = async () => {
    const initSignature = await program.methods
      .initiate(swapAmount, expiresInSlots, bob.publicKey, [...secretHash])
      .accounts({
        initiator: alice.publicKey,
      })
      .signers([alice])
      .rpc({ commitment: "confirmed" });
    console.log("Alice initiated:", initSignature);
  };

  before(async () => {
    rentAmount = await connection.getMinimumBalanceForRentExemption(
      program.account.swapAccount.size
    );
    let signature: string;
    const blockHash = await connection.getLatestBlockhash();
    console.log("Fund alice with 1 SOL");
    signature = await connection.requestAirdrop(
      alice.publicKey,
      1 * web3.LAMPORTS_PER_SOL
    );
    await connection.confirmTransaction({ signature, ...blockHash });
  });

  it("Test initiation", async () => {
    const alicePreBalance = await connection.getBalance(alice.publicKey);

    await aliceInitiate();

    const pdaBalance = await connection.getBalance(swapAccount);
    expect(pdaBalance).to.equal(rentAmount + swapAmount.toNumber());

    const alicePostBalance = await connection.getBalance(alice.publicKey);
    expect(alicePostBalance).to.equal(
      alicePreBalance - swapAmount.toNumber() - rentAmount
    );
  });

  it("Test redeem", async () => {
    const bobPreBalance = await connection.getBalance(bob.publicKey);

    // The previous test has already initiated the swap
    const redeemSignature = await program.methods
      .redeem([...secret])
      .accounts({
        swapAccount,
        initiator: alice.publicKey,
        redeemer: bob.publicKey,
      })
      .rpc();
    console.log("Bob redeemed:", redeemSignature);

    const bobPostBalance = await connection.getBalance(bob.publicKey);
    expect(bobPostBalance).to.equal(bobPreBalance + swapAmount.toNumber());

    const pdaBalance = await connection.getBalance(swapAccount);
    expect(pdaBalance).to.equal(0);
  });

  it("Test refund", async () => {
    await aliceInitiate(); // Initiate again for the test

    const alicePreBalance = await connection.getBalance(alice.publicKey);

    console.log("Awaiting timelock for refund");
    await setTimeout(expiresInSlots.toNumber() * 400 + 500);

    const refundSignature = await program.methods
      .refund()
      .accounts({
        swapAccount,
        initiator: alice.publicKey,
      })
      .rpc({ commitment: "confirmed" });
    console.log("Alice refunded:", refundSignature);

    const alicePostBalance = await connection.getBalance(alice.publicKey);
    expect(alicePostBalance).to.equal(
      alicePreBalance + swapAmount.toNumber() + rentAmount
    );

    const pdaBalance = await connection.getBalance(swapAccount);
    expect(pdaBalance).to.equal(0);
  });

  it("Test instant refund", async () => {
    await aliceInitiate(); // Initiate again for the test

    const alicePreBalance = await connection.getBalance(alice.publicKey);

    const instantRefundSignature = await program.methods
      .instantRefund()
      .accounts({
        swapAccount,
        initiator: alice.publicKey,
        redeemer: bob.publicKey,
      })
      .signers([bob])
      .rpc();
    console.log("Alice instant-refunded:", instantRefundSignature);

    const alicePostBalance = await connection.getBalance(alice.publicKey);
    expect(alicePostBalance).to.equal(
      alicePreBalance + swapAmount.toNumber() + rentAmount
    );

    const pdaBalance = await connection.getBalance(swapAccount);
    expect(pdaBalance).to.equal(0);
  });
});
