# LEGAL.md

Legal, privacy, accessibility and trust rules for this project. Any AI
coding agent working on this repository follows this file.

Scope: EU law (GDPR, ePrivacy, consumer law, Accessibility Act, AI Act) and
US law (COPPA, CAN-SPAM, California rules, FTC rules). The developer is
based in Poland (EU); users may be anywhere.

This file is engineering guidance, not legal advice. It reflects research
current to October 2026. Anything marked "get qualified advice" goes to a
lawyer before launch in a paid market.

## Hard rules (never break)

1. The privacy policy describes what the product actually does, written
   from the real data inventory. A mismatch between text and behaviour is
   the finding.
2. Nothing optional (analytics, ads, marketing) loads or records before
   consent, where consent is required.
3. Marketing email always carries a working unsubscribe and a postal
   address.
4. Subscription renewal terms sit next to the purchase button, not in a
   FAQ.
5. If the app can be used by children or is aimed at them, the age gate
   and COPPA/GDPR-Article-8 rules apply before any data collection.
6. No pre-ticked boxes, no fake countdowns, no hidden fees, no dark
   patterns, no fake reviews, no unsupported claims. Anywhere.
7. Every image, font, icon, sound and code snippet has a licence that
   covers how this project uses it.
8. Session-replay and analytics tooling never records keystrokes or form
   input.

## 1. Legal documents and disclosures

- Privacy policy: list every category of personal data collected by the
  app, backend, logs and third-party tools (including device identifiers
  and IP addresses); state purpose and lawful basis per category (EU/UK
  GDPR Art. 13/14); name recipients (hosting, analytics, email, payment,
  AI providers); state retention periods or the criteria; explain user
  rights (access, deletion, correction, portability, objection, opt-out)
  and how to exercise them; explain international transfers and the
  safeguard (e.g. EU-US Data Privacy Framework, SCCs); add an effective
  date and keep previous versions. California: keep the notice-at-collection
  and "Do Not Sell or Share" requirements in mind; review at least every
  12 months while CCPA applies. Link it from the footer, sign-up flow,
  settings and store listings; it loads without login and is not blocked
  by the cookie banner.
- Terms of service: service description, eligibility and minimum age,
  acceptable use, user-content licence, pricing/billing/refunds/cancellation
  consistent with the actual flows, liability and dispute clauses reviewed
  per market (get qualified advice), change-notification rule. Presented at
  sign-up and purchase with a clear acceptance step; versioned.
- Acceptance checkbox: unticked checkbox or a clear "By creating an account
  you agree to..." next to the button. Pre-ticked boxes are not valid GDPR
  consent (CJEU Planet49). The privacy policy is acknowledged ("I have
  read the Privacy Policy"), not consented to as a bundle. Optional
  marketing consent is a separate control.
- Acceptance records: append-only storage of user ID, document version,
  timestamp with timezone, and the mechanism (checkbox, screen). Archived
  copy of every version exactly as shown, plus optional consents given or
  refused. A test user's accepted version must be retrievable.
- Refund policy: which plans are refundable, time limits, how to request,
  processing time. EU/UK: 14-day withdrawal right for distance contracts,
  with the digital-content exception only where express consent and
  acknowledgement were collected; EU traders need an online withdrawal
  function for online distance contracts (from 19 June 2026). Store
  in-app purchases are refunded through Apple/Google; say so.
- Business contact details: legal entity name, geographic address, working
  contact email, registration and VAT numbers where required (EU
  e-Commerce Directive Art. 5, German Impressum, DSA contact points,
  CAN-SPAM postal address in commercial email). Reachable from every page
  and identical across terms, privacy policy, invoices, emails and store
  listings. Verify: a test message through each published route gets an
  answer.

## 2. Consent, cookies and marketing

- Inventory every cookie, local-storage key, pixel and SDK identifier with
  provider, purpose and lifetime; classify strictly necessary vs optional.
- Cookie banner (EU/UK): consent before non-essential storage; "Accept
  all" and "Reject all" with equal prominence; granular per purpose;
  cookie notice naming providers and retention; persistent way to change
  or withdraw; consent record with banner version and timestamp. A banner
  that only says "we use cookies" with OK is not valid consent.
- Trackers stay off until consent: optional scripts and SDKs load only
  after the consent manager allows them; tag managers default to off;
  withdrawal stops tracking. US state laws: notice and opt-out,
  honouring Global Privacy Control where required.
- Marketing consent is separate, unticked, per channel (email/SMS/push),
  never a condition of service. Stored with wording, source, date, synced
  to every marketing tool before sending. EU/UK electronic marketing
  needs prior consent (narrow soft opt-in for similar products to existing
  customers); Canada CASL consent; US CAN-SPAM is opt-out but TCPA is
  strict for texts and calls.
- Every marketing email: visible unsubscribe link working without login,
  processed within 10 business days, valid at least 30 days after sending;
  List-Unsubscribe and one-click headers through the provider; valid
  physical postal address for US recipients; preference centre with a
  global "unsubscribe from all". Suppression list so unsubscribed
  addresses are never re-imported. Marketing sends separated from
  transactional sends (receipts, security alerts keep working).
- Verify: in a clean browser reject everything; in DevTools no optional
  cookies or tracking requests fire. Unsubscribe a test address; nothing
  reaches it from any tool afterwards.

## 3. Handling personal data

- Data minimisation: every form field, permission request, SDK data point
  and log field has a written purpose. Remove fields with no purpose;
  make optional fields clearly optional; ask permissions at the moment of
  need with an explanation; prefer less precise data (age range over
  birth date, approximate location); never log passwords, tokens,
  payment data or health details. Verify against real network traffic.
- Retention: a schedule per data category with period and reason, checked
  against legal minimums (invoices, tax); automated deletion or
  anonymisation jobs covering databases, file storage and search indexes;
  retention settings applied in third-party tools; backups covered (how
  deleted data ages out); disclosed in the privacy policy. Test one
  record through its full lifecycle.
- Access and deletion requests: at least one easy intake route (in-app
  form or privacy email). GDPR: respond within one month (extendable by
  two). CCPA: within 45 days. Identity verification proportionate, not
  excessive. Build an export in a readable format and in-app account
  deletion that removes or anonymises across database, storage and third
  parties (Apple and Google require in-app deletion for account-creating
  apps; Google also wants a web route). Pass deletion to processors. Log
  each request and outcome. Document exceptions (invoices) and tell the
  user what was kept.
- Children: decide with evidence whether the app is directed to children
  or likely used by them. US COPPA (under 13): verifiable parental
  consent before collection, minimum data until consent, direct notice to
  parents, separate consent for third-party disclosures, high-privacy
  defaults (no geolocation, profiling or public profiles), parent review
  and deletion tools; the amended COPPA Rule applies since 22 April 2026.
  EU/UK GDPR Art. 8: parental consent below 16, or a lower national age
  not below 13 (UK and Ireland: 13; check member-state rules per market).
  Minimum implementation when children are possible: an age gate at
  sign-up; keep it proportionate and document the choice (simple
  "I am 13 or older" confirmation vs date of birth: get qualified advice
  when the app targets minors). No tracking, advertising or analytics
  data about a child leaves the app before consent.
- AI training on user data needs its own lawful basis and disclosure.
  Disable vendor training on our data in every AI provider's settings
  where possible; never train on business customers' data unless their
  contract permits; separate training datasets from production data.
  Record the vendor check.

## 4. Vendors and third parties

- Every SDK, script, pixel and widget is inventoried with version, owner,
  data collected, destination and reason. Capture real network traffic
  from app and website; check whether URLs, page titles, form fields or
  event names leak sensitive data (health, financial). Disable vendor
  features not used (advertising identifiers, session replay, automatic
  capture). Remove SDKs that cannot be justified; re-run the traffic
  capture after each release that changes SDKs. Undisclosed sharing has
  drawn FTC enforcement (health data through ad pixels).
- Session replay and analytics: never record keystrokes or form input
  content; keep it off by default; consent rules above apply in full.
- Data processing agreements: a written DPA with every processor
  (GDPR Art. 28), terms covering our markets, signed copies kept, vendor
  terms must not allow advertising or AI training on our users' data.
  Transfers safeguarded (SCCs or adequacy). If this project processes
  data for business customers, publish a subprocessor list at a stable
  URL with change notifications.

## 5. Accessibility

Standard: WCAG 2.2 level AA. Benchmarks for the European Accessibility
Act (in application since 28 June 2025 for covered services; exemption
for microenterprises providing services) and ADA case law. Get qualified
advice when selling into regulated markets.

- Contrast: at least 4.5:1 for normal text, 3:1 for large text and for
  meaningful UI components and graphics; every state (hover, pressed,
  error, disabled, dark mode); fix colour tokens in the design system,
  not individual screens; never colour alone conveys meaning.
- Alt text on every meaningful image; decorative images explicitly empty;
  icon-only controls get accessible names; charts get text alternatives;
  require alt text (or a decorative flag) for user-uploaded images.
- Keyboard: everything reachable by keyboard in a logical order with a
  visible focus indicator; no keyboard traps; focus managed in dialogs;
  skip-to-content link on long pages; sticky headers and banners never
  hide focus.
- Verify: run sign-up, checkout and cancellation using only Tab,
  Shift+Tab, Enter, Space, Escape and arrows; automated checker plus one
  manual screen-reader pass on key flows.

## 6. Fair design and pricing

- No dark patterns: "yes" and "no" choices comparable in size, placement
  and wording; neutral decline copy (no confirmshaming); add-ons and
  optional permissions unselected by default; full price and key terms
  visible before commitment; advertising labelled.
- No fake urgency: timers and scarcity messages must be driven by real
  deadlines and real stock; remove anything that resets on reload or
  persists past its stated end.
- No hidden fees: the first price shown includes all mandatory fees and
  taxes; optional extras opt-in; breakdown and total before payment
  details; a test purchase charges exactly the first total shown.
- Auto-renewal disclosure: renewal fact, renewal price, billing frequency
  and cancellation method displayed next to the purchase button; free
  trial conversion terms (when and how much) stated; express agreement
  to renewal terms obtained before charging; acknowledgement email
  repeating the terms; renewal reminders and price-change notices where
  law or platform rules require (US ROSCA; California ARL as amended
  from 1 July 2025; EU/UK pre-purchase information rules; Apple and
  Google have their own).
- Easy cancellation: sign-up online means cancel online, in the same app
  or site, without calls or chats; clearly labelled in account or billing
  settings; retention offers limited, never blocking cancellation;
  cancellation stops future charges and is confirmed; status synced
  between billing provider and database. App-store subscriptions link to
  the platform's management page. Time the flow and compare it with
  sign-up.

## 7. Honest claims

- Every objective claim (results, speed, savings, security, AI
  capability, comparisons) has dated evidence kept before publishing.
  Statistics state source, sample and date. Remove or rewrite claims
  without support; qualify typical-results claims.
- Reviews and testimonials trace to real customers. No invented, staff,
  AI-written or undisclosed-incentivised reviews; negatives shown by the
  same rules, not suppressed; FTC Rule 16 CFR Part 465 and UK DMCCA ban
  fake review practices; EU requires describing how reviews are checked.
  Never buy reviews or pay for positivity.
- No false compliance claims: SOC 2, ISO, HIPAA, GDPR claims only with a
  current report or certificate of matching scope, described precisely
  ("Type 2 report available under NDA for period X"), never "certified"
  without one, never implying a provider's report covers this app.
- Uptime claims match measured reality and provider SLAs; no "100%
  uptime, guaranteed"; publish an honest status page; monitor
  availability independently.
- Customer logos only with written permission, kept on file with scope
  and date; removed when the relationship ends; wording never implies
  endorsement or partnership beyond "customer".
- AI-generated or AI-edited content: label it visibly where users could
  be misled; disclose chatbots as AI (EU AI Act Art. 50 transparency
  applies since 2 August 2026; synthetic media marking follows Commission
  guidance); keep labels through downloads and shares where feasible;
  follow platform AI-disclosure rules.

## 8. Intellectual property

- Every asset (images, icons, illustrations, fonts, audio, video, copied
  code) has a recorded source, licence type, holder and proof. Licences
  must cover the actual use: commercial use in our media, web and app
  font embedding (desktop licences do not cover web/app embedding),
  logo use. CC non-commercial assets never ship in commercial products.
  Attribution pages where required. AI-generated assets: check the
  tool's terms; the US Copyright Office position is that material
  without human authorship is not copyrightable, which limits our own
  rights.
- Open source: keep an SBOM (dependencies with versions and licences);
  permissive licences (MIT, BSD, Apache-2.0) still need notices, licence
  texts and Apache NOTICE contents, e.g. an "Open-source licences"
  screen; copyleft (GPL, LGPL, AGPL) can require source availability
  depending on distribution model; decide with advice before release,
  especially for anything built on GPL code. Automated licence checks in
  the build pipeline.
- DMCA (US safe harbor when hosting user-uploaded content): register a
  designated agent in the Copyright Office DMCA directory (fee, renews
  every 3 years), publish the agent's contact on the site and in terms,
  publish a copyright policy with notice and counter-notice steps, adopt
  a repeat-infringer policy, build a takedown workflow that acts fast and
  notifies the uploader. Calendar the renewal.

## High-cost per-unit risks (fixed penalties per user, session or email)

These are the failure modes that turn zero revenue into a large
liability from a single user, session or email. Checked before every
launch:

| Risk | Rule | Verify |
|---|---|---|
| One underage user | Age gate at sign-up; COPPA path if minors are possible | Sign-up blocks under-13 path per the documented choice |
| Fonts leaking visitor IP to a third-party CDN (EU court penalties, e.g. Munich) | Fonts self-hosted on this domain | Network tab: zero third-party font requests |
| Session replay recording keystrokes (California wiretapping risk) | Replay and analytics never record form input; off by default; consent first | Type into every form with recording on: no input captured |
| Marketing email without unsubscribe + postal address | Both present and working in every commercial email | Unsubscribe works without login; address renders in plain-text and HTML |
| Renewal terms not disclosed at the purchase button | Terms, price, frequency, cancel method next to the button | New-user read-through: visible without opening another page |
| User uploads without a registered DMCA agent | Agent registered and published | Registration current (3-year renewal) and contact on site |

## Final review before launch

- Lawyer review of privacy policy, terms, refund and subscription flows
  and marketing claims for each paid market (get qualified advice).
- Re-run this file's verify steps on the final release build, not a dev
  build.
- Public documents, store listings and the live product describe the same
  behaviour.
- Each section has an owner and a next review date; re-run relevant
  sections when a feature, vendor, data category, pricing model or market
  is added.

## Sources (named law where agents need to check further)

GDPR (EU 2016/679), UK GDPR / DPA 2018, ePrivacy Directive 2002/58/EC,
PECR, EDPB consent guidelines, CCPA/CPRA, CalOPPA, CAN-SPAM, TCPA, CASL,
WCAG 2.2, European Accessibility Act (EU 2019/882), UCPD 2005/29/EC, UK
DMCCA 2024, FTC Act §5, FTC rules on fees (16 CFR 464), reviews (16 CFR
465) and endorsements (16 CFR 255), ROSCA, California Automatic Renewal
Law, COPPA Rule 16 CFR 312 (amended 2026), EU AI Act (EU 2024/1689)
Art. 50, Consumer Rights Directive 2011/83/EU and (EU) 2023/2673,
e-Commerce Directive 2000/31/EC, DSA (EU 2022/2065), E-SIGN, DMCA 17
U.S.C. §512 and 37 CFR §201.38, Lanham Act §43(a), CJEU Planet49,
Berman v. Freedom Financial Network (9th Cir. 2022).

## References (full bibliography)

Codes match the reference list printed in the source checklist; R41 and
R43 do not exist there. This supplements the law summary above.

- **R1** — Regulation (EU) 2016/679 (GDPR), Articles 5, 6, 7, 8, 12-17, 20, 25, 28. EUR-Lex: eur-lex.europa.eu.
- **R2** — UK GDPR and Data Protection Act 2018; ICO guidance; Data (Use and Access) Act 2025. ico.org.uk.
- **R3** — Directive 2002/58/EC (ePrivacy), Art. 5(3) and Art. 13. EUR-Lex.
- **R4** — Privacy and Electronic Communications Regulations 2003 (PECR); ICO guidance on storage and access technologies and direct marketing. legislation.gov.uk; ico.org.uk.
- **R5** — EDPB Guidelines 05/2020 on consent under the GDPR. European Data Protection Board: edpb.europa.eu.
- **R6** — CCPA as amended by the CPRA (Cal. Civ. Code §1798.100 et seq.) and CCPA Regulations (Cal. Code Regs. tit. 11, §7000 et seq.). California Privacy Protection Agency: cppa.ca.gov.
- **R7** — CalOPPA, Cal. Bus. & Prof. Code §§22575-22579. California Legislative Information: leginfo.legislature.ca.gov.
- **R8** — CAN-SPAM Act (15 U.S.C. §7701 et seq.), 16 CFR Part 316; FTC CAN-SPAM Compliance Guide for Business. ftc.gov.
- **R9** — WCAG 2.2, W3C Recommendation. w3.org/TR/WCAG22.
- **R10** — Directive (EU) 2019/882 (European Accessibility Act, applicable from 28 June 2025); harmonised standard EN 301 549. EUR-Lex.
- **R11** — Directive 2005/29/EC (Unfair Commercial Practices), as amended by Directive (EU) 2019/2161, incl. Annex I. EUR-Lex.
- **R12** — Digital Markets, Competition and Consumers Act 2024 (Part 4 and Schedule 20); CMA guidance. legislation.gov.uk; gov.uk/government/organisations/competition-and-markets-authority.
- **R13** — FTC staff report "Bringing Dark Patterns to Light" (September 2022). ftc.gov.
- **R14** — FTC Act §5 (15 U.S.C. §45); FTC Policy Statement Regarding Advertising Substantiation. ftc.gov.
- **R15** — FTC Guides Concerning the Use of Endorsements and Testimonials in Advertising, 16 CFR Part 255. ecfr.gov.
- **R16** — FTC Trade Regulation Rule on the Use of Consumer Reviews and Testimonials, 16 CFR Part 465. ecfr.gov.
- **R17** — Regulation (EU) 2024/1689 (AI Act), Art. 50; European Commission FAQ on Art. 50 transparency obligations. EUR-Lex; digital-strategy.ec.europa.eu.
- **R18** — Directive 2011/83/EU (Consumer Rights); Directive (EU) 2023/2673 (online withdrawal function, applicable from 19 June 2026). EUR-Lex.
- **R19** — Consumer Contracts (Information, Cancellation and Additional Charges) Regulations 2013. legislation.gov.uk.
- **R20** — Restore Online Shoppers' Confidence Act (ROSCA), 15 U.S.C. §§8401-8405. US Code (law.cornell.edu).
- **R21** — California Automatic Renewal Law, Cal. Bus. & Prof. Code §§17600-17606, as amended by AB 2863 (effective 1 July 2025). leginfo.legislature.ca.gov.
- **R22** — Children's Online Privacy Protection Rule, 16 CFR Part 312 (amended rule; compliance date 22 April 2026). ftc.gov.
- **R23** — ICO "Age appropriate design: a code of practice for online services" (Children's code). ico.org.uk.
- **R24** — Electronic Signatures in Global and National Commerce Act (E-SIGN), 15 U.S.C. §7001 et seq. US Code.
- **R25** — Open Source Initiative licence list; SPDX License List; GNU licences FAQ; Apache License 2.0. opensource.org; spdx.org; gnu.org; apache.org.
- **R26** — AICPA System and Organization Controls: SOC suite, incl. SOC 2 and Trust Services Criteria. aicpa-cima.com.
- **R27** — 17 U.S.C. §512; 37 CFR §201.38; DMCA Designated Agent Directory. copyright.gov/dmca-directory.
- **R28** — FTC "AI (and other) Companies: Quietly Changing Your Terms of Service Could Be Unfair or Deceptive" (2024); EDPB Opinion 28/2024 on AI models. ftc.gov; edpb.europa.eu.
- **R29** — Apple App Review Guidelines (5.1.1 data collection and account deletion; 3.1.2 subscriptions) and privacy manifests; Google Play Developer Policy (User Data, account deletion, Data safety, subscriptions). developer.apple.com; support.google.com.
- **R30** — Commission Implementing Decision (EU) 2021/914 on Standard Contractual Clauses. EUR-Lex.
- **R31** — FTC Rule on Unfair or Deceptive Fees, 16 CFR Part 464 (effective 12 May 2025); Cal. Civ. Code §1770(a)(29) (SB 478). ecfr.gov; leginfo.legislature.ca.gov.
- **R32** — Directive 2000/31/EC (e-Commerce Directive) Art. 5; Regulation (EU) 2022/2065 (Digital Services Act); UK Electronic Commerce (EC Directive) Regulations 2002; Companies (Trading Disclosures) Regulations 2008. EUR-Lex; legislation.gov.uk.
- **R33** — Global Privacy Control specification; CCPA Regulations §7025 on opt-out preference signals. globalprivacycontrol.org; cppa.ca.gov.
- **R34** — U.S. Copyright Office, Copyright and Artificial Intelligence (registration guidance and reports). copyright.gov/ai.
- **R35** — Lanham Act §43(a), 15 U.S.C. §1125(a) (false designation and false endorsement). US Code.
- **R36** — Case C-673/17, Planet49 GmbH, CJEU judgment, 1 October 2019. CURIA (curia.europa.eu).
- **R37** — Berman v. Freedom Financial Network, LLC, 30 F.4th 849 (9th Cir. 2022), on online contract formation. Ninth Circuit.
- **R38** — CNIL, "Cookies and other trackers: guidance and recommendations". cnil.fr.
- **R39** — Custom Communications, Inc. v. FTC (8th Cir., 8 July 2025), vacating the amended Negative Option Rule. Eighth Circuit.
- **R40** — DMCCA 2024, Part 4 Chapter 2 (subscription contracts); commencement reported April 2026 as expected in spring 2027. legislation.gov.uk; gov.uk.
- **R42** — FTC enforcement action against GoodRx (2023) on health data shared through advertising tools; Washington My Health My Data Act (RCW 19.373). ftc.gov; Washington State Legislature (leg.wa.gov).
- **R44** — U.S. Department of Justice, Guidance on Web Accessibility and the ADA. ada.gov.
- **R45** — Digitale-Dienste-Gesetz (DDG) §5, general information obligations (Impressum). Federal Ministry of Justice, Germany: gesetze-im-internet.de.

Abbreviations: GDPR = EU General Data Protection Regulation; ICO = UK
Information Commissioner's Office; EDPB = European Data Protection Board;
CCPA = California Consumer Privacy Act (as amended by the CPRA);
CalOPPA = California Online Privacy Protection Act; CASL = Canadian
Anti-Spam Legislation; UCPD = EU Unfair Commercial Practices Directive;
DMCCA = UK Digital Markets, Competition and Consumers Act 2024;
ROSCA = Restore Online Shoppers' Confidence Act; EAA = European
Accessibility Act; DSA = Digital Services Act; DPA = data processing
agreement; CNIL = Commission nationale de l'informatique et des libertés.

Basis: The App Launch Checklist (35 checks, research to October 2026);
App Legal Guard PRD (per-unit risk list).
