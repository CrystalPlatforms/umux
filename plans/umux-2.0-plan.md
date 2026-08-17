# umux 2.0 — decyzje z discovery (sierpień 2026)

> Dokument referencyjny z sesji `/ask` (17.08.2026). Formalne zmiany wpisane są do
> [`umux-prd.md`](./umux-prd.md); ten plik zachowuje kontekst: analizę konkurencji,
> uzasadnienia decyzji i plan promocji.

## Kontekst i cel

Adam (product owner) chce rozwinąć umux tak, aby konkurował z **herdr**
(https://github.com/herdrdev/herdr, ~30k gwiazdek) i zbudował zasięg w social mediach.
Cele: **więcej gwiazdek GitHub i pobrań**, produkt bez znanych problemów herdra.

## Analiza konkurencji — herdr

**Czym jest:** multiplexer terminali w stylu tmux dla agentów AI (Claude Code, Codex,
Cursor…). Rust, pojedynczy binarny plik, sterowanie klawiszami w stylu tmux + mysz.
Instalacja: curl / Homebrew / mise / PowerShell (Windows w betcie).

**Mocne strony:**
- trwały serwer w tle — sesje przeżywają restart komputera i odłączenie,
- status agenta w każdym panelu (pracuje / czeka / bezczynny),
- agenci mogą sami sterować herdr przez CLI i socket API,
- system pluginów z marketplace,
- ~30k gwiazdek, bardzo aktywna społeczność.

**Znane problemy herdra (wg issue, posortowane po głosach) — nasza lista rzeczy do uniknięcia:**
| Problem herdra | Nasza odpowiedź w umux |
|---|---|
| Limit 60Hz renderowania (#1134) | WriteBatcher + xterm.js — pilnujemy przepustowości |
| Powiadomienie, które nigdy nie znika (#1827) | Nasze powiadomienia działają — **nie zmieniamy** (decyzja Adama) |
| Mylący status agenta („complete" zamiast „in-progress", #1217) | Status agenta w umux ma być dokładny — kryterium akceptacji |
| Agenci niewykrywani w panelu (#803, #1170) | Wykrywanie wyłącznie przez OSC — proste, przewidywalne |
| Niespójne potwierdzenia zamykania (#1750) | Jedna zasada: panel z procesem → zawsze pytanie; pusty → nigdy |
| Linki łamane między wierszami (#1282) | xterm.js obsługuje — weryfikujemy przy testach |

**Pozycjonowanie umux:** natywne **GUI** dla wszystkich — prostota „klikasz i działa"
dla osób nieznających tmux + funkcje klasy herdr dla zaawansowanych. Nie kopiujemy
modelu „terminal dla terminala".

## Decyzje

### Produkt (v0.2.0, najpierw Linux)
1. **Panele bez limitu** (dotąd max 2) — wymaga przepisania PaneLayout na drzewo podziałów.
2. **Status agenta per panel** (pracuje / czeka / bezczynny) z parsowanego strumienia OSC.
3. **Ekran Ustawień** z przełącznikami funkcji (status agenta, powiadomienia, analityka) —
   persystowane w WorkspaceStore.
4. **Przywracanie sesji**: po otwarciu umux wracają workspace'y, panele, układ, katalogi
   i powłoki. Procesy NIE są wznawiane (brak serwera w tle — to v2.0).
5. **Spójne zamykanie**: panel z działającym procesem → zawsze potwierdzenie; pusty → nigdy.
6. **Aptabase** — anonimowa analityka (darmowy limit 20k zdarzeń/mies., SDK pod Tauri,
   możliwość wyłączenia w Ustawieniach).
7. **Powiadomienia bez zmian** — działają dobrze na Ubuntu; tylko weryfikujemy na Win/mac.

### Platformy i CI (v1.0.0)
- Linux (.deb + .AppImage) + **Windows (NSIS `-setup.exe`)** + **macOS (universal `.dmg`:
  Apple Silicon + Intel w jednym pliku)**.
- Buildy **niepodpisane** (polityka zero kosztów) + instrukcja pierwszego uruchomienia
  w README (macOS: prawy klik → Otwórz / `xattr -cr`; Windows: SmartScreen → „Więcej
  informacji → Uruchom mimo to").
- Różnice per system: powłoka domyślna (`$SHELL`/bash vs PowerShell), katalog konfiguracji
  (`~/.config/umux` / `%APPDATA%\umux` / `~/Library/Application Support/umux`).
- **SSH tylko Linux + macOS** na v1.0.0 (Windows później — różnice agenta/kluczy).
- CI: GitHub Actions przy publikacji release'a (jak dziś), dokładamy 2 joby
  (`windows-latest`, `macos-latest` z targetem `universal-apple-darwin`).
- Testowanie: Adam na swoim sprzęcie (Mac lokalnie + komputer z Windows).

### Wersjonowanie
- **v0.2.0** — wszystkie nowe funkcje, Linux.
- **v1.0.0** — Windows + macOS + CI 3 platformy + landing + start promocji.
- **v2.0 (później)** — system pluginów z marketplace oraz opcjonalny serwer w tle
  (pełna persistencja sesji jak herdr).

### Promocja (start razem z v1.0.0)
- Kanały: **X/Twitter + dev.to** (YouTube odkładamy na potem — przy budżecie ≤1 h/tydz.
  lepiej skupić się tam, gdzie zasięg przychodzi taniej). Język: **wyłącznie angielski**.
- Materiały (screeny, GIF-y, teksty) **tworzy Adam**; Claude dostarcza listę darmowych
  narzędzi i checklistę, co nagrać.
- Strona: landing na **Cloudflare Pages** (`umux.pages.dev`, 0 zł) — downloady, badge'y
  shields.io, GIF demo.

#### Darmowe narzędzia (free-for.dev, sprawdzone 08.2026)
| Narzędzie | Zastosowanie | Limit darmowy |
|---|---|---|
| Cloudflare Pages | landing umux | 500 buildów/mies. |
| Aptabase | analityka w aplikacji (SDK Tauri) | 20k zdarzeń/mies. |
| Buttondown / Maildroppa | newsletter / lista oczekujących | 100 subskrybentów |
| Metashot | obrazki OG pod posty | 1000 renderów/mies. |
| Pika Code Screenshots | zrzuty kodu na posty | darmowe |
| GoatCounter | statystyki strony (bez cookies) | 100k odsłon/mies. |
| newreleases.io | powiadomienia o wydaniach dla userów | darmowe |
| shields.io | badge'y (wersja, licencja, pobrania) | darmowe |
| OBS Studio / Kap | nagrywanie ekranu na GIF/wideo | darmowe (open source) |
| Canva | grafiki prostych elementów | darmowe |

### Mierzalny sukces
- GWIAZDKI: wzrost względem dnia startu promocji (punkt odniesienia: stan przy v1.0.0).
- POBRANIA: licznik pobrań z GitHub Releases (per platforma).
- UŻYCIE: zdarzenia Aptabase (aktywne użycie, nie tylko pobrania).

## Otwarte kwestie / założenia przyjęte bez pytania
- Pluginy i serwer w tle — **po v1.0.0** (Adam chciał pluginów; zapisane w Roadmapie v2.0
  z uwagi na rozmiar pracy).
- YouTube — wraca do rozmowy po starcie promocji, jeśli starczy czasu.
- Kolejność prac: **najpierw funkcje (v0.2.0), potem buildy i CI (v1.0.0), na końcu
  promocja** — zgodnie z decyzją Adama.
