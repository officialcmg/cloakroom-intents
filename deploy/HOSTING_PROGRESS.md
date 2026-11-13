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
