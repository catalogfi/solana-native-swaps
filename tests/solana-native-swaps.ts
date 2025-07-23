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

  const sponsor = new web3.Keypair();
  const funder = new web3.Keypair();

  // SwapAccount PDA
  const pdaSeeds = [
    expiresInSlots.toArrayLike(Buffer, "le", 8),
    alice.publicKey.toBuffer(),
    bob.publicKey.toBuffer(),
    secretHash,
    swapAmount.toArrayLike(Buffer, "le", 8),
  ];
  const [swapAccount] = web3.PublicKey.findProgramAddressSync(
    pdaSeeds,
    program.programId
  );
  const destinationData = crypto.randomBytes(256); // can be null
  let rentAmount: number;

  console.log({ alice: alice.publicKey, bob: bob.publicKey, swapAccount });

  const aliceInitiate = async () => {
    const initSignature = await program.methods
      .initiate(
        expiresInSlots,
        bob.publicKey,
        [...secretHash],
        swapAmount,
        destinationData
      )
      .accounts({
        initiator: alice.publicKey,
        sponsor: sponsor.publicKey,
      })
      .signers([alice, sponsor])
      .rpc({ commitment: "confirmed" });
    console.log("Alice initiated:", initSignature);
  };

  before(async () => {
    rentAmount = await connection.getMinimumBalanceForRentExemption(
      program.account.swapAccount.size
    );

    const blockHash = await connection.getLatestBlockhash();
    const fund = async (to: web3.PublicKey, qty: number) => {
      const signature = await connection.requestAirdrop(
        to,
        qty * web3.LAMPORTS_PER_SOL
      );
      await connection.confirmTransaction({ signature, ...blockHash });
    };
    console.log("Fund alice with 1 SOL");
    await fund(alice.publicKey, 1);
    console.log("Fund sponsor with 0.1 SOL");
    await fund(sponsor.publicKey, 0.1);
    console.log("Fund funder with 1 SOL");
    await fund(funder.publicKey, 1);
  });

  it("Test initiate on behalf", async () => {
    const alicePreBalance = await connection.getBalance(alice.publicKey);
    const funderPreBalance = await connection.getBalance(funder.publicKey);
    const sponsorPreBalance = await connection.getBalance(sponsor.publicKey);

    const initiateOnBehalfSignature = await program.methods
      .initiateOnBehalf(
        expiresInSlots,
        alice.publicKey,
        bob.publicKey,
        secretHash,
        swapAmount,
        null
      )
      .accounts({
        funder: funder.publicKey,
        sponsor: sponsor.publicKey,
      })
      .signers([sponsor, funder])
      .rpc();
    console.log(
      "Funder initiated on behalf of alice:",
      initiateOnBehalfSignature
    );

    const pdaBalance = await connection.getBalance(swapAccount);
    expect(pdaBalance).to.equal(rentAmount + swapAmount.toNumber());

    const alicePostBalance = await connection.getBalance(alice.publicKey);
    expect(alicePostBalance).to.equal(alicePreBalance);

    const funderPostBalance = await connection.getBalance(funder.publicKey);
    expect(funderPostBalance).to.equal(
      funderPreBalance - swapAmount.toNumber()
    );

    const sponsorPostBalance = await connection.getBalance(sponsor.publicKey);
    expect(sponsorPostBalance).to.equal(sponsorPreBalance - rentAmount);
  });

  it("Test redeem", async () => {
    const bobPreBalance = await connection.getBalance(bob.publicKey);
    const sponsorPreBalance = await connection.getBalance(sponsor.publicKey);

    // The previous test has already initiated the swap
    const redeemSignature = await program.methods
      .redeem([...secret])
      .accounts({
        swapAccount,
        sponsor: sponsor.publicKey,
        redeemer: bob.publicKey,
      })
      .rpc();
    console.log("Bob redeemed:", redeemSignature);

    const bobPostBalance = await connection.getBalance(bob.publicKey);
    expect(bobPostBalance).to.equal(bobPreBalance + swapAmount.toNumber());

    const pdaBalance = await connection.getBalance(swapAccount);
    expect(pdaBalance).to.equal(0);

    const sponsorPostBalance = await connection.getBalance(sponsor.publicKey);
    expect(sponsorPostBalance).to.equal(sponsorPreBalance + rentAmount);
  });

  it("Test refund", async () => {
    await aliceInitiate(); // Initiate again for the test

    const alicePreBalance = await connection.getBalance(alice.publicKey);
    const sponsorPreBalance = await connection.getBalance(sponsor.publicKey);

    console.log("Awaiting timelock for refund");
    await setTimeout(expiresInSlots.toNumber() * 400 + 500);

    const refundSignature = await program.methods
      .refund()
      .accounts({
        swapAccount,
        initiator: alice.publicKey,
        sponsor: sponsor.publicKey,
      })
      .rpc({ commitment: "confirmed" });
    console.log("Alice refunded:", refundSignature);

    const alicePostBalance = await connection.getBalance(alice.publicKey);
    expect(alicePostBalance).to.equal(alicePreBalance + swapAmount.toNumber());

    const pdaBalance = await connection.getBalance(swapAccount);
    expect(pdaBalance).to.equal(0);

    const sponsorPostBalance = await connection.getBalance(sponsor.publicKey);
    expect(sponsorPostBalance).to.equal(sponsorPreBalance + rentAmount);
  });

  it("Test instant refund", async () => {
    await aliceInitiate(); // Initiate again for the test

    const alicePreBalance = await connection.getBalance(alice.publicKey);
    const sponsorPreBalance = await connection.getBalance(sponsor.publicKey);

    const instantRefundSignature = await program.methods
      .instantRefund()
      .accounts({
        swapAccount,
        initiator: alice.publicKey,
        redeemer: bob.publicKey,
        sponsor: sponsor.publicKey,
      })
      .signers([bob])
      .rpc();
    console.log("Alice instant-refunded:", instantRefundSignature);

    const alicePostBalance = await connection.getBalance(alice.publicKey);
    expect(alicePostBalance).to.equal(alicePreBalance + swapAmount.toNumber());

    const pdaBalance = await connection.getBalance(swapAccount);
    expect(pdaBalance).to.equal(0);

    const sponsorPostBalance = await connection.getBalance(sponsor.publicKey);
    expect(sponsorPostBalance).to.equal(sponsorPreBalance + rentAmount);
  });
});
