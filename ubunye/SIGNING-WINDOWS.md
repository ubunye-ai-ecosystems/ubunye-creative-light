# Signing the Windows builds

This guide explains how to make the Windows installer and `.exe` of Ubunye Light show our name
instead of "Unknown publisher". Written 2026-10-11. Check the links again before you pay for anything,
because the rules change often.

## What a signature does

Windows checks every downloaded program for a code signature.

* **Unsigned:** Windows shows "Windows protected your PC" and "Unknown publisher". People must click
  "More info" and then "Run anyway". Many will give up.
* **Signed:** the warning names the publisher. The SmartScreen warning can still appear for the first
  weeks, until enough people have downloaded the signed file. This is normal for every new publisher.

## What the build already does

The release workflow (`.github/workflows/release.yml`, job for Windows) calls
`packaging/windows/sign.ps1` on the `.exe` and the `.msi`. That script knows two ways to sign:

| Way | Secrets it reads |
| --- | --- |
| A certificate file (`.pfx`) | `WINDOWS_CERTIFICATE` (the file in base64), `WINDOWS_CERTIFICATE_PASSWORD` |
| Azure Artifact Signing | `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE` |

With no secrets set, the build still works and the files stay unsigned (you get a warning, not a
failure). The secrets live in the GitHub **environment** called `release`, which only the `release`
branch can use.

## The catch: neither built in way fits us today

1. **Azure Artifact Signing** (the cheapest paid option, about USD 10 a month) is not open to South
   Africa. Organisations must be in the US, Canada, EU, UK or a short list of other countries.
   Individual developers must be in the US or Canada.
   ([Microsoft quickstart](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart))
2. **A `.pfx` file** no longer exists for new certificates. Since June 2023 every public certificate
   authority must keep the signing key on hardware (a USB token or a cloud key store). You cannot
   download it as a file, so the `WINDOWS_CERTIFICATE` path only works for old certificates.

So whichever option you pick below, `sign.ps1` or `release.yml` needs a small change. Our rules
(`ubunye/CLAUDE.md`, rule 8) say signing changes need your approval first, so that change is not made yet.

## Option 1 (recommended): SignPath Foundation, free

[SignPath Foundation](https://signpath.org/) signs open source projects for free from GitHub Actions.
Our repositories are public and MIT or Apache-2.0, so we can apply.

Their conditions ([terms](https://signpath.org/terms)):

* OSI open source licence, no paid dual licence. **We pass.**
* No closed source parts. **We pass:** our icons in `ubunye/brand/` are MIT or Apache-2.0, and the
  build removes upstream's ArtCraft logos (the one non open source item, see `assets/ATTRIBUTION.md`).
* Actively maintained and **already released** in the form to sign. We need at least one public
  (unsigned) release on GitHub first.
* A **code signing policy** page on our website that names who can approve a signature.
  A page at `thabangvision.com/tools/code-signing` would do.
* The build must be fully automatic from the source in the repository. **We pass** (release.yml).

The signature will say **"SignPath Foundation"** as publisher, not Ubunye. That is the price of free.

Steps:

1. Make one public GitHub release of each app (unsigned is fine).
2. Publish the code signing policy page (the SignPath site has a template).
3. Apply at [signpath.org/apply](https://signpath.org/apply.html), once per app or once for all three.
4. When accepted, SignPath gives you an organisation id, a project slug and an API token. Put the
   token in the `release` environment as the secret `SIGNPATH_API_TOKEN`.
5. Approve the workflow change: the Windows job uploads the unsigned files with SignPath's GitHub
   action and downloads the signed ones. After that, `sign.ps1` is only used for the old ways.

## Option 2: buy a certificate with a cloud key

Use this if you want **Ubunye AI Ecosystems** (or your own name) as the publisher.

1. **Pick the type.**
   * *Organisation (OV):* needs a registered company (CIPC number), a listed phone number and a
     working address. The publisher shows the company name.
   * *Individual (IV):* some authorities sell these. The publisher shows your full legal name.
2. **Pick an authority that offers cloud signing**, so GitHub Actions can sign without a USB token.
   For example [SSL.com eSigner](https://www.ssl.com/guide/esigner-codesigntool-command-guide/) or
   Certum SimplySign. Compare prices; expect a yearly fee of a few hundred US dollars.
3. **Pass the identity check.** They will ask for ID, company documents and sometimes a phone call.
   It takes from a day to two weeks.
4. **Save the login details as secrets** in the `release` environment. For SSL.com eSigner:
   `ES_USERNAME`, `ES_PASSWORD`, `ES_CREDENTIAL_ID`, `ES_TOTP_SECRET`.
5. Approve the change that adds this third way to `sign.ps1`.

## Adding secrets to the release environment

On GitHub: the repository, then **Settings**, **Environments**, **release**, **Add environment
secret**. Do it in each of the three repositories (Light, Photo, Film), or ask Claude to do it.

## Checking a signed build

After a release run, download the `.msi` and in PowerShell run:

```powershell
Get-AuthenticodeSignature .\lightcraft-<version>-windows-x64.msi | Format-List Status, SignerCertificate
```

`Status` must be `Valid`. Then right click the file, **Properties**, **Digital Signatures**, and check
the name and the time stamp.

## Other platforms

macOS needs an Apple Developer account (USD 99 a year) and the `APPLE_*` secrets in the same
environment. That is a separate decision.
