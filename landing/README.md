# Landing page (issue #35, Phase 11)

Statyczna strona `index.html` — zero frameworków, zero build-stepu. Testy:
`npm test -- landing` (Vitest parsuje HTML i sprawdza linki/badge'y/sekcje).

## 1. Podgląd lokalnie

```bash
cd landing && python3 -m http.server 8080
```

Otwórz http://localhost:8080. Zobaczysz placeholder „real file pending"
zamiast GIF-a/screenshotów — normalne, dopóki nie wrzucisz plików (niżej).

## 2. Wrzuć swoje materiały (tylko kopiowanie plików!)

Wrzuć pliki do `landing/assets/` **pod tymi dokładnie nazwami** — HTML nie
wymaga wtedy żadnych zmian (placeholders znikną same):

| Plik | Co wrzucasz |
|---|---|
| `assets/demo.gif` | demo GIF 20–30 s (cel: ≤ 8 MB) |
| `assets/screenshot-linux.png` | zrzut z Linuxa (Wayland) |
| `assets/screenshot-macos.png` | zrzut z macOS |
| `assets/screenshot-windows.png` | zrzut z Windows |

Już wrzucone: `assets/umux-logo.png` (nagłówek) i `assets/umux-favicon.ico`
(ikona karty przeglądarki) — skopiowane z `public-assets/` 2026-08-27.

**Zachowania interaktywne** (mały skrypt inline, zero zewnętrznych bibliotek):
- „Download for Linux" rozwija listę plików; po wybraniu formatu wyskakuje
  dialog z komendą instalacji (chmod +x / apt install / dnf install).
- „Download for Windows" i „Download for macOS" po kliknięciu pokazują
  dialog z podziękowaniem i życzeniami.
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

## 4. Statystyki GoatCounter (opcjonalne)

1. Załóż darmowe konto na https://goatcounter.com (do 100 tys. wejść/mies.).
2. Dodaj stronę `umux.pages.dev` — dostaniesz **kod witryny** (słowo przed `.goatcounter.com`).
3. W `landing/index.html`, tuż nad `</body>`, odkomentuj blok `<!-- GoatCounter … -->`
   i podmień `YOUR-SITE-CODE` na swój kod. Zapis, commit — strona zaktualizuje się sama.

Bez tego kroku strona w ogóle nie wysyła żadnych danych (zero skryptów, zero cookies).

## 5. Nowa wersja aplikacji = podmiana numeru

Linki używają wzorca GitHuba `releases/latest/download/<nazwa>` — zawsze
wskazują najnowszy release, ale **nazwy plików zawierają numer wersji**.
Po każdym release podmień numer w 5 miejscach w `index.html`
(na dziś: `1.0.2` → np. `1.0.3`):

```
umux_1.0.2_amd64.AppImage
umux_1.0.2_amd64.deb
umux-1.0.2-1.x86_64.rpm
umux_1.0.2_universal.dmg
umux_1.0.2_x64-setup.exe
```

Test Ci przypomni, jeśli któraś nazwa się rozjedzie z tym, co produkuje CI
(wzorzec potwierdzony w `removed-marketing-drafts`). Archiwum aktualizatora
(`umux_universal.app.tar.gz`) celowo nie ma linku — nie linkujemy.

## 6. Checklist HITL (przed publikacją promocji)

- [ ] Materiały wrzucone do `assets/` (GIF + 3 screenshoty)
- [ ] Strona zdeployowana na `umux.pages.dev`
- [ ] Otwórz stronę **na telefonie**: hero widać bez scrollowania, docelowo
      **2 tapy** prowadzą do pobierania (tap „Download for…" → potwierdzenie)
- [ ] „Download for Linux" → dropdown → wybór pliku → dialog z instrukcją
- [ ] „Download for Windows"/"macOS" → dialog z podziękowaniem
- [ ] Logo w nagłówku i favicon karty się renderują
- [ ] Badge'y (release / license / downloads) się renderują
- [ ] Po włączeniu GoatCounter: wejścia widoczne w panelu
- [ ] Dopiero potem: promocja (`removed-marketing-drafts` — wątek X, dev.to)
