//TODO: UPDATE TEST FILE

const assert = require("assert");
const { web3, BN } = require("@project-serum/anchor");
const { SystemProgram } = web3;

describe("Merkle Liquidity Pool", () => {
  // Create keypairs for pool, mint, and users
  const poolKp = new web3.Keypair();
  const userKp = new web3.Keypair();
  const mintLpToken = new web3.Keypair();
  
  const initialRoot = Buffer.alloc(32); // Simulate a Merkle root for LPs (all zeros for test)

  let poolAccount = null;
  let userTokenAccountA = null;
  let userTokenAccountB = null;

  before(async () => {
    // Airdrop SOL to the user so they can pay for transactions
    await pg.connection.requestAirdrop(userKp.publicKey, 1e9);
    
    // Create token accounts for Token A and Token B for the user
    userTokenAccountA = await pg.createTokenAccount();
    userTokenAccountB = await pg.createTokenAccount();

    // Initialize the pool
    await pg.program.methods
      .initializePool(initialRoot, mintLpToken.publicKey)
      .accounts({
        pool: poolKp.publicKey,
        user: userKp.publicKey,
        systemProgram: SystemProgram.programId,
        mintLpToken: mintLpToken.publicKey,
      })
      .signers([poolKp, userKp, mintLpToken])
      .rpc();
  });

  it("initializes the liquidity pool", async () => {
    // Fetch the pool data
    poolAccount = await pg.program.account.pool.fetch(poolKp.publicKey);
    
    // Check if the pool's initial data is correctly set
    assert.strictEqual(poolAccount.tokenA_Balance.toNumber(), 0);
    assert.strictEqual(poolAccount.tokenB_Balance.toNumber(), 0);
    assert.strictEqual(poolAccount.mintLpToken.toBase58(), mintLpToken.publicKey.toBase58());
    assert.strictEqual(poolAccount.merkleRoot.toString("hex"), initialRoot.toString("hex"));
    assert.strictEqual(poolAccount.swapFee.toNumber(), 30); // default 0.3%
  });

  it("swaps tokens", async () => {
    const amountIn = new BN(1000); // User sends 1000 tokens

    // Create a simulated Merkle proof for LP validation
    const merkleProof = [
      Buffer.alloc(32), // Simulate the proof as an array of 32-byte buffers (mocked)
    ];

    const userHash = web3.keccak256(
      Buffer.concat([userKp.publicKey.toBuffer(), Buffer.from(amountIn.toArray())])
    );

    // Swap tokens from Token A to Token B
    await pg.program.methods
      .swapTokens(amountIn, merkleProof, initialRoot)
      .accounts({
        pool: poolKp.publicKey,
        user: userKp.publicKey,
        tokenAccountA: userTokenAccountA, // User's Token A account
        tokenAccountB: userTokenAccountB, // User's Token B account
        tokenProgram: web3.TOKEN_PROGRAM_ID,
      })
      .signers([userKp])
      .rpc();

    // Fetch updated pool data after the swap
    poolAccount = await pg.program.account.pool.fetch(poolKp.publicKey);

    // Confirm token A balance increased and token B balance decreased
    assert.strictEqual(poolAccount.tokenA_Balance.toNumber(), amountIn.toNumber());
    // Token B balance would decrease (assuming there was liquidity) – here, we're simplifying
  });

  it("claims liquidity with valid Merkle proof", async () => {
    const amountToClaim = new BN(500); // User wants to claim 500 tokens
    const merkleProof = [Buffer.alloc(32)]; // Simulated proof

    // Claim liquidity using a valid Merkle proof
    await pg.program.methods
      .claimLiquidity(merkleProof, initialRoot, amountToClaim)
      .accounts({
        pool: poolKp.publicKey,
        user: userKp.publicKey,
        poolTokenAccount: poolKp.publicKey, // pool's token account
        userTokenAccount: userTokenAccountA, // User's Token A account
        tokenProgram: web3.TOKEN_PROGRAM_ID,
      })
      .signers([userKp])
      .rpc();

    // Fetch updated pool data after claiming liquidity
    poolAccount = await pg.program.account.pool.fetch(poolKp.publicKey);

    // Pool's token A balance should have decreased by 500 tokens
    assert.strictEqual(poolAccount.tokenA_Balance.toNumber(), 500);
  });

  it("performs partial withdrawal", async () => {
    const withdrawAmount = new BN(200); // LP wants to withdraw 200 tokens
    const merkleProof = [Buffer.alloc(32)]; // Simulated proof

    await pg.program.methods
      .partialWithdraw(merkleProof, initialRoot, withdrawAmount)
      .accounts({
        pool: poolKp.publicKey,
        user: userKp.publicKey,
        poolTokenAccount: poolKp.publicKey, // Pool's token account
        userTokenAccount: userTokenAccountA, // User's Token A account
        tokenProgram: web3.TOKEN_PROGRAM_ID,
      })
      .signers([userKp])
      .rpc();

    // Fetch the updated pool account
    poolAccount = await pg.program.account.pool.fetch(poolKp.publicKey);

    // Confirm that the pool balance has been reduced by the partial withdrawal amount
    assert.strictEqual(poolAccount.tokenA_Balance.toNumber(), 300); // Was 500, now 300 after withdrawal
  });

  it("emergency withdrawal with penalty", async () => {
    const withdrawAmount = new BN(100); // LP emergency withdraws 100 tokens
    const merkleProof = [Buffer.alloc(32)]; // Simulated proof

    await pg.program.methods
      .emergencyWithdraw(merkleProof, initialRoot, withdrawAmount)
      .accounts({
        pool: poolKp.publicKey,
        user: userKp.publicKey,
        poolTokenAccount: poolKp.publicKey, // Pool's token account
        userTokenAccount: userTokenAccountA, // User's Token A account
        tokenProgram: web3.TOKEN_PROGRAM_ID,
      })
      .signers([userKp])
      .rpc();

    // Fetch the updated pool account
    poolAccount = await pg.program.account.pool.fetch(poolKp.publicKey);

    // Emergency withdrawal with a 10% penalty means 90 tokens were withdrawn, pool reduced by 90
    assert.strictEqual(poolAccount.tokenA_Balance.toNumber(), 210); // Was 300, minus 90
  });
});
