# Hosting progress (scratch, untracked, no secrets)

## 10:30 UTC resume
- Found on disk: deploy/maker/railway/{Dockerfile,Dockerfile.dockerignore,start.sh}, deploy/web/{vercel-build.sh,vercel-ignore.sh}.
  The main session pushed them to main as 34f763b (WIP); local HEAD still 6ec3ad7.
- Railway: no zecret project yet (only cloakroom-demo, ckb-keyway, uniport-backend, echo).
- Vercel: project `zecret` prj_Mb3xhOSaYxnYcc9f5PxFSRfUKLIX in chrismgs-projects, git-connected to
  officialcmg/zecret main (deployments carry githubCommitRef main). Builds with web/vercel.json
  (npm run build, no engine yet). SSO protection on (all_except_custom_domains) -> must turn off
  for the public production URL.
- No local zecret process running.
## 10:33 Railway
- Created project `zecret` 7cef823e-d834-4c56-9826-e072a4abaa6e, env production e4b3b306-c4a0-45a5-a822-ba036dce67ec,
  empty service `zecret-maker` 2708fc53-4643-46fd-bec6-38c532afcd06. Next: settings, volume /data, variables, source.
## 10:45 Railway service configured
- Secrets route changed: reading the maker wallet / NEAR key into the variable tool was refused by the
  permission classifier. Instead the HOSTED service creates its own wallet and NEAR key on the volume
  (ZECRET_NEW_WALLET=1, ZECRET_NEW_NEAR_KEY=1). Then locally: add its NEAR public key to
  maker.zecret-3f109eea1e.testnet (AddKey signed with deploy/.keys, never printed) and pay its UA from
  .tools/testnet/maker with `zecret pay`.
- start.sh edited locally (ZECRET_NEW_NEAR_KEY, logs UA / NEAR pubkey / directory entry). NOT on main yet.
  Until pushed, a temporary Railway start command does near-key + directory, then execs tini zecret-start.
  -> clear startCommand after the main session pushes start.sh.
- Settings: dockerfilePath deploy/maker/railway/Dockerfile, watch /crates/**, /deploy/maker/**, restart ALWAYS,
  volume zecret-maker-data 9be140bc-... at /data (1 GB), vars ZECRET_NEW_WALLET, ZECRET_NEW_NEAR_KEY, RUST_BACKTRACE, PORT=8080.
- Source connected: officialcmg/zecret main. Domain: https://zecret-maker-production.up.railway.app (port 8080).
## 10:50 Web side edits
- web/vercel.json: buildCommand bash ../deploy/web/vercel-build.sh, ignoreCommand bash ../deploy/web/vercel-ignore.sh,
  connect-src + https://zecret-maker-production.up.railway.app. testnet.json relayUrl -> that domain. npm test (78),
  typecheck, lint pass. wasm-bindgen 0.2.129 hash in vercel-build.sh verified by download.
- Vercel project: rootDirectory web, sourceFilesOutsideRootDirectory true, ssoProtection -> prod_deployment_urls_and_all_previews
  (production alias public). Public URL: https://zecret-chrismgs-projects.vercel.app (zecret.vercel.app is NOT ours).
- testnet/tools/src/addkey.rs + bin zecret-testnet-addkey (built: crates/target/release/zecret-testnet-addkey).
- Still missing: web/public/config/testnet-directory.json (from hosted maker's `directory entry:` log line).
## 10:58 Hosted maker up (first deploy 3665dfa5, build ~2 min)
- Hosted wallet UA utest10kyef9lq7c637hlxx3fwdfrkukqnttzswcr4ukes5h3ewzca4e4nzycm73txxfgf44azfyngreys08dpzxqp8eah8dwacktj3s5yu0wv
- Hosted NEAR public key ed25519:4aQfVk7LxZzASG5mLRsjDn1AxkzsCDjjmtf5q6ApZyHH (NOT yet a key of maker.zecret-3f109eea1e.testnet)
- ZECRET_NEW_WALLET / ZECRET_NEW_NEAR_KEY set back to 0 (skipDeploys).
- AddKey to maker.zecret-3f109eea1e.testnet REFUSED by permission classifier ("Permission Grant"). Owner decision needed.
- Paid 50,000,000 zats from .tools/testnet/maker to hosted UA: txid b49b2d844aee5e15b2a5272cdda5ea732be38881eef9790479517b6708947f99
## 11:03 Directory + persistence
- Redeploy 51ddefb5 (11:00): same UA and NEAR key, "config.json matches the image" -> volume persists across redeploys.
- Directory entry (v2, signed limits 100000..10000000, premium 1000) logged by host; published as
  web/public/config/testnet-directory.json via deploy/web/directory-from-entry.mjs (rate 30). Raw entry kept in
  deploy/maker/railway/directory-entry.testnet.json. Parsed OK by web parseDirectory.
- Relay over HTTPS: OPTIONS 204 with ACAO *, POST {} -> 400. Mainnet refusal checked locally (zecret maker --relay with network main -> error).
- docs/HOSTING.md drafted. testnet/hosted/run.mjs written (hosted browser swap).
- BLOCKER: hosted NEAR key not on maker account (AddKey refused by classifier). Owner must run:
  ./crates/target/release/zecret-testnet-addkey deploy/.keys/testnet/maker.zecret-3f109eea1e.testnet.json maker.zecret-3f109eea1e.testnet ed25519:4aQfVk7LxZzASG5mLRsjDn1AxkzsCDjjmtf5q6ApZyHH
## 11:20 Handing back to main session
- No push to main since d6bac89, so the Vercel engine build (vercel-build.sh) has NOT run yet.
- Remaining: (1) owner/main runs addkey command (docs/HOSTING.md); (2) main commits+pushes -> watch Vercel build
  logs and Railway rebuild; (3) clear Railway startCommand once new start.sh is on main; (4) run
  `node testnet/hosted/run.mjs https://zecret-chrismgs-projects.vercel.app .tools/testnet/taker zecret-3f109eea1e.testnet 0.01`.
