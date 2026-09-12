# Landing page (issue #35, Phase 11)

Statyczna strona `index.html` — zero frameworków, zero build-stepu. Testy:
`npm test -- landing` (Vitest parsuje HTML i sprawdza linki/badge'y/sekcje).

## 1. Podgląd lokalnie

```bash
cd landing && python3 -m http.server 8080
```

Otwórz http://localhost:8080. Zobaczysz placeholder „real file pending"
zamiast GIF-a/screenshotów — normalne, dopóki nie wrzucisz plików (niżej).

## 2. Materiały graficzne (już na stronie)

Media żyją w `landing/assets/` pod własnymi nazwami.
**Podmiana = skopiowanie nowego pliku pod tą samą nazwą**
(HTML bez zmian):

| Plik | Gdzie się pokazuje |
|---|---|
| `assets/umux-first.png` | duży obraz w hero (główne okno: workspaces, taby, panele) |
| `assets/umux-agent.png` | karta „Agent status & notifications" |
| `assets/umux-session.png` | karta „Session restore" |
| `assets/og.jpg` | podgląd linku na X/Discord/LinkedIn (meta `og:image`, 1200×630) |
| `assets/umux-logo.png` | logo w nagłówku |
| `assets/umux-favicon.ico` | ikona karty przeglądarki |

Jeśli obrazek nie istnieje, w jego miejscu pojawia się ładny placeholder
(mechanizm `onerror`) — nigdy „rozbity" obrazek.

**Zachowania interaktywne** (mały skrypt inline, zero zewnętrznych bibliotek):
- „Download for Linux" rozwija listę plików; po wybraniu formatu wyskakuje
  dialog z komendą instalacji (chmod +x / apt install / dnf install).
- „Download for Windows" i „Download for macOS" po kliknięciu pokazują
  dialog z podziękowaniem i życzeniami.
- Sekcja „Install the CLI": przycisk **Copy** kopiuje komendę (feedback
  „Copied!"), a finalny przycisk CTA na dole sam dobiera platformę
  (Windows/macOS z przeglądarki; Linux → strona release'u).
- Animacje: sekcje delikatnie wjeżdżają przy scrollu (klasa `.reveal`);
  bez JS i przy `prefers-reduced-motion` wszystko widać od razu.
Teksty dialogów edytujesz w `landing/index.html` w bloku `<script>` na dole
(zmienne `thanks` i `install`).

Zalecenie: screenshoty w tej samej rozdzielczości (np. 2400×1500), każdy z
innym motywem/projektem — widać, że to naprawdę natywne buildy.
Jeśli GIF wyjdzie cięższy niż 8 MB: `ffmpeg -i demo.gif -vf "fps=12,scale=1200:-1" demo-small.gif`.

## 3. Deployment na Cloudflare Pages (darmowe, ~3 minuty)

1. Wejdź na https://dash.cloudflare.com → **Workers & Pages** → **Create** → zakładka **Pages** → **Connect to Git**.
2. Połącz GitHubem i wybierz repo **CrystalPlatforms/umux**.
3. W „Set up builds and production deployments" ustaw:
   - **Project name:** `umux` (z tego wyjdzie adres `umux.pages.dev`)
   - **Production branch:** `main`
   - **Framework preset:** `None`
   - **Build command:** *(zostaw puste)*
   - **Build output directory:** `landing`
4. **Save and Deploy**. Po ~30 s strona żyje pod `https://umux.pages.dev`.

Kolejne commity do `main` publikują stronę automatycznie (nic nie robiś).

## 4. Statystyki GoatCounter (AKTYWNE — kod `crystalstudio`)

Skrypt licznika jest już wpisany w `landing/index.html` (nad `</body>`) i
aktywny od 2026-08-27. Panel statystyk: **https://crystalstudio.goatcounter.com**.

Do zrobienia w panelu (jednorazowo):
1. **Settings → Data collection → domains**: wpisz `umux.pages.dev`
   (a po podpięciu domeny — też ją), żeby nie liczyły się wejścia z localhost.
2. Test: otwórz stronę, wróć do panelu — pierwsze wejście widać w ~1 min.

Uwagi: zero cookies (bez banerów RODO); osoby z adblockiem nie są liczone;
darmowy plan wymaga niekomercyjnego użytku (do 100 tys. wejść/mies.).

## 5. Nowa wersja aplikacji = podmiana numeru

Linki używają wzorca GitHuba `releases/latest/download/<nazwa>` — zawsze
wskazują najnowszy release, ale **nazwy plików zawierają numer wersji**.
Po każdym release podmień numer w `index.html`
(na dziś: `1.0.4` → np. `1.0.5`) — w linkach download **i w tekstach
dialogów** dla Linuksa (te same nazwy plików):

```
umux_1.0.4_amd64.AppImage
umux_1.0.4_amd64.deb
umux-1.0.4-1.x86_64.rpm
umux_1.0.4_universal.dmg
umux_1.0.4_x64-setup.exe
```

Test Ci przypomni, jeśli któraś nazwa się rozjedzie z tym, co produkuje CI
(wzorzec potwierdzony z CI przy v1.0.2). Archiwum aktualizatora
(`umux_universal.app.tar.gz`) celowo nie ma linku — nie linkujemy.

## 6. Checklist HITL (przed publikacją promocji)

- [ ] Materiały na stronie (hero + 2 karty z obrazkami — gotowe 2026-08-27)
- [ ] Strona zdeployowana na `umux.pages.dev`
- [ ] Otwórz stronę **na telefonie**: hero widać bez scrollowania, docelowo
      **2 tapy** prowadzą do pobierania (tap „Download for…" → potwierdzenie)
- [ ] „Download for Linux" → dropdown → wybór pliku → dialog z instrukcją
- [ ] „Download for Windows"/"macOS" → dialog z podziękowaniem
- [ ] Logo w nagłówku i favicon karty się renderują
- [ ] Badge'y (release / license / downloads) się renderują
- [ ] Po włączeniu GoatCounter: wejścia widoczne w panelu
- [ ] Dopiero potem: promocja (wątek X, dev.to)
