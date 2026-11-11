#!/usr/bin/env node
// Puts an uploaded build into the public external TestFlight group through the App Store Connect API, so one
// upload run reaches link testers without a click in the web console.
//   1. waits until the build has finished processing (it shows up a few minutes after the upload)
//   2. adds it to the public group
//   3. submits it for beta app review unless it already has a submission; a build of an already approved
//      marketing version is normally approved within minutes
// Credentials stay outside the repository: ~/.appstoreconnect/issuer_id and the single
// ~/.appstoreconnect/private_keys/AuthKey_<KEY_ID>.p8 (ASC_ISSUER_ID / ASC_KEY_PATH override them).
// Usage: node apps/ios/scripts/testflight-public.mjs <build-number> [--status]
//   --status   only print the build's processing, group and review state, change nothing
import { createSign } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const APP_ID = process.env.ASC_APP_ID ?? "6827766462";
const GROUP_ID = process.env.ASC_PUBLIC_GROUP_ID ?? "8caac48f-6fec-4c81-bde2-a937904e2d1d";
const WAIT_MINUTES = Number(process.env.ASC_WAIT_MINUTES ?? 60);

const [buildNumber, flag] = process.argv.slice(2);
if (!buildNumber || !/^\d+$/.test(buildNumber)) {
  console.error("usage: testflight-public.mjs <build-number> [--status]");
  process.exit(2);
}
const statusOnly = flag === "--status";

const ascDir = join(homedir(), ".appstoreconnect");
const issuer = (process.env.ASC_ISSUER_ID ?? readFileSync(join(ascDir, "issuer_id"), "utf8")).trim();
const keyPath =
  process.env.ASC_KEY_PATH ??
  join(ascDir, "private_keys", readdirSync(join(ascDir, "private_keys")).find((f) => /^AuthKey_.+\.p8$/.test(f)));
const keyId = keyPath.match(/AuthKey_(.+)\.p8$/)[1];
const privateKey = readFileSync(keyPath, "utf8");

// ES256 JWT; App Store Connect accepts at most 20 minutes, so a fresh one is minted per request.
function token() {
  const b64 = (v) => Buffer.from(JSON.stringify(v)).toString("base64url");
  const now = Math.floor(Date.now() / 1000);
  const body = `${b64({ alg: "ES256", kid: keyId, typ: "JWT" })}.${b64({ iss: issuer, iat: now, exp: now + 600, aud: "appstoreconnect-v1" })}`;
  const sig = createSign("SHA256").update(body).sign({ key: privateKey, dsaEncoding: "ieee-p1363" });
  return `${body}.${sig.toString("base64url")}`;
}

async function api(method, path, body) {
  const res = await fetch(`https://api.appstoreconnect.apple.com${path}`, {
    method,
    headers: { Authorization: `Bearer ${token()}`, "Content-Type": "application/json" },
    body: body && JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${method} ${path} -> ${res.status} ${text}`);
  return text ? JSON.parse(text) : null;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function findBuild() {
  const q = `filter[app]=${APP_ID}&filter[version]=${buildNumber}&include=betaGroups,betaAppReviewSubmission&limit=1`;
  const res = await api("GET", `/v1/builds?${q}`);
  const build = res.data[0];
  if (!build) return null;
  const included = res.included ?? [];
  return {
    id: build.id,
    processing: build.attributes.processingState,
    inGroup: (build.relationships.betaGroups?.data ?? []).some((g) => g.id === GROUP_ID),
    review: included.find((i) => i.type === "betaAppReviewSubmissions")?.attributes.betaReviewState ?? null,
  };
}

function show(b) {
  console.log(`build ${buildNumber}: processing ${b.processing}, public group ${b.inGroup ? "yes" : "no"}, beta review ${b.review ?? "none"}`);
}

// The build appears in the API only some minutes after the upload, then sits in PROCESSING.
const deadline = Date.now() + WAIT_MINUTES * 60_000;
let build = await findBuild();
while (!statusOnly && (!build || build.processing === "PROCESSING")) {
  if (Date.now() > deadline) throw new Error(`build ${buildNumber} still not processed after ${WAIT_MINUTES} min`);
  console.log(build ? `build ${buildNumber} processing…` : `build ${buildNumber} not visible yet…`);
  await sleep(30_000);
  build = await findBuild();
}
if (!build) {
  console.log(`build ${buildNumber}: not found`);
  process.exit(statusOnly ? 0 : 1);
}
show(build);
if (statusOnly) process.exit(0);
if (build.processing !== "VALID") throw new Error(`build ${buildNumber} ended in ${build.processing}`);

if (!build.inGroup) {
  await api("POST", `/v1/betaGroups/${GROUP_ID}/relationships/builds`, { data: [{ type: "builds", id: build.id }] });
  console.log("added to the public group");
}
if (!build.review) {
  await api("POST", "/v1/betaAppReviewSubmissions", {
    data: { type: "betaAppReviewSubmissions", relationships: { build: { data: { type: "builds", id: build.id } } } },
  });
  console.log("submitted for beta app review");
}
show(await findBuild());
