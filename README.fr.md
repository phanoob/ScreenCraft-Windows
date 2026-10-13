# ScreenCraft

[English](README.md) · **Français**

Un enregistreur d'écran et éditeur vidéo pour macOS et Windows, écrit en Rust. Enregistrez
votre écran, une fenêtre ou une zone (avec, au choix, une bulle caméra, le micro et le
son système), puis transformez la capture brute en une vidéo soignée : zooms
automatiques fluides qui suivent votre curseur et votre frappe, curseur redessiné,
arrière-plans, coins arrondis, ombres, coupes, changements de vitesse, musique de fond
et export MP4 / GIF.

Aucune fonction d'IA : seulement l'enregistrement, la caméra, le montage et le zoom.

> **Portage Windows de l'application macOS d'origine.** ScreenCraft a été créé par
> **Marshall Nickolauson** pour macOS :
> [MarshallNickolauson/ScreenCraft](https://github.com/MarshallNickolauson/ScreenCraft)
> (licence MIT, voir [LICENSE](LICENSE)). Ce dépôt conserve tout son historique et son
> code macOS sans modification, et y ajoute une partie Windows (`src-tauri/src/win`)
> ainsi que les quelques adaptations de l'interface nécessaires sous Windows. Le mérite
> de l'application, de sa conception et de la version macOS revient à l'auteur
> d'origine ; les problèmes liés au portage Windows se signalent ici, pas sur le dépôt
> d'origine.

> **Sans affiliation.** ScreenCraft est un projet indépendant et ouvert, *inspiré* de
> l'idée de [Screen Studio](https://screen.studio). Il n'est ni affilié à Screen Studio
> ou à ses créateurs, ni approuvé par eux, ni lié à eux d'aucune façon. « Screen
> Studio » est une marque de son propriétaire. ScreenCraft ne contient aucun code,
> visuel ou autre ressource de ce produit.

## Captures d'écran

L'éditeur, avec un enregistrement de démonstration généré (une « application »
factice avec une zone de saisie, pas de vraies images). Les zones de zoom sont les
blocs bleus de la timeline.

![L'éditeur](docs/screenshots/editor.jpg)

En pause pendant un zoom : la caméra cadre les cartes et l'aperçu recharge une image
en pleine résolution pour que les arrêts sur image zoomés restent nets.

![Zoom dans l'éditeur](docs/screenshots/editor-zoom.jpg)

Réglages du curseur et des mouvements de caméra :

![Curseur et mouvements de caméra](docs/screenshots/editor-cursor.jpg)

L'export, avec compression facultative après l'export et une estimation de la taille :

![Fenêtre d'export](docs/screenshots/export.jpg)

La barre d'enregistrement : mode de capture, caméra / micro / son système avec
indicateurs de niveau, résolution et fréquence d'images, et la liste des projets.

![Barre d'enregistrement](docs/screenshots/recorder.jpg)

## Ce qu'il fait

**Enregistrement**
- Capture d'un écran entier, d'une seule fenêtre ou d'une zone tracée à la souris (tout
  ce qui est autour est assombri pour bien voir ce qui est enregistré).
- Bulle caméra, micro et son système, chacun avec un choix de périphérique et un
  indicateur de niveau en direct. Choix de la résolution (Native / 2560 / 1920 / 1600 /
  1280) et de 30 ou 60 i/s.
- Le curseur du système n'est pas incrusté dans la vidéo : il peut être redessiné,
  lissé et restylé dans l'éditeur. Les clics, les mouvements du curseur et les frappes
  au clavier sont enregistrés à côté.
- Bandeau d'enregistrement déplaçable, compte à rebours, raccourci global **⌘⇧2**
  (**Ctrl+Shift+2** sous Windows), commandes dans la barre de menus (zone de
  notification sous Windows).

**Montage**
- **Zoom :** zones de zoom générées automatiquement à partir de vos clics et de votre
  frappe, avec entrée / sortie réglables, plus des zooms manuels à placer n'importe
  où. La caméra garde le curseur près du centre et, pendant la frappe, suit le curseur
  de texte.
- **Timeline :** pistes vidéo, zoom et musique. Couper, raccourcir, réordonner,
  dupliquer et supprimer des clips ; vitesse réglable par clip ; zones de zoom et clips
  de musique déplaçables librement.
- **Apparence :** fonds d'écran, dégradés, couleurs unies ou votre propre image (avec
  flou), marge, arrondi des coins, ombre, format d'image, recadrage.
- **Curseur :** plusieurs styles, effets de clic, masquage à l'arrêt, masquage pendant
  la frappe.
- **Bulle caméra :** taille, forme, position, miroir, bordure ; elle rétrécit en
  douceur pendant les zooms.
- **Son :** niveaux séparés pour le micro, l'appareil, la musique et le général. Le
  micro est nivelé et centré automatiquement.
- **Musique :** ajout d'un fichier audio, raccourci et découpe, forme d'onde affichée
  sur la piste.
- **Fichiers de projet :** les modifications restent en mémoire jusqu'à
  l'enregistrement (**⌘S**, **Ctrl+S** sous Windows) ; un `*` dans le titre signale
  des modifications non enregistrées, et fermer ou quitter demande avant de les perdre.

**Export et compression**
- MP4 (H.264 / HEVC) ou GIF de 720p à 4K, en 30 / 60 i/s, avec estimation de la taille.
- *Compression après l'export* facultative (préréglages à la HandBrake, via
  AVFoundation sur macOS et Media Foundation sous Windows), avec le choix de garder
  l'original, la copie compressée, ou les deux.
- Un outil **Compresser une vidéo…** indépendant, pour n'importe quel fichier vidéo.

**Langues**
- L'interface existe en anglais, en français et en allemand. Elle suit la langue du
  système ; pour la changer, utilisez **Langue** dans le menu Projets (barre
  d'enregistrement ou éditeur).

## Installation

ScreenCraft se compile à partir des sources ; il n'existe pas encore de version signée
à télécharger.

### macOS

**Prérequis**
- macOS 15 (Sequoia) ou plus récent (développé et testé sur Apple silicon)
- [Rust](https://rustup.rs) (stable) et les outils en ligne de commande Xcode
  (`xcode-select --install`)

```bash
git clone https://github.com/MarshallNickolauson/ScreenCraft.git
cd ScreenCraft

./scripts/bundle.sh --install     # compile une version release et la copie dans ~/Applications
open ~/Applications/ScreenCraft.app
```

Sans `--install`, `./scripts/bundle.sh` laisse l'application dans
`dist/ScreenCraft.app`. La première compilation release prend quelques minutes.

**Autorisations.** Au premier lancement, une fenêtre de configuration vérifie les
autorisations macOS nécessaires et ouvre le bon volet des Réglages Système :

| Autorisation | Pourquoi |
| --- | --- |
| Enregistrement de l'écran | capturer l'écran, les fenêtres et les zones (requis) |
| Accessibilité | trouver le curseur de texte pour que la caméra suive la frappe (requis) |
| Surveillance de l'entrée | détecter le début de la frappe (requis) |
| Caméra / Micro | seulement pour la bulle caméra ou la narration |

macOS n'applique l'enregistrement de l'écran et la surveillance de l'entrée qu'après
le redémarrage de l'application ; la fenêtre propose un bouton pour redémarrer.
L'application est signée ad hoc avec une identité stable, donc les autorisations sont
conservées d'une compilation à l'autre.

### Windows

**Prérequis**
- Windows 10 version 2004 ou plus récent (Windows 11 recommandé : il peut masquer la
  bordure jaune de capture)
- [Rust](https://rustup.rs) (stable) et les Visual Studio Build Tools avec la charge
  de travail « Développement Desktop en C++ »
- Le runtime WebView2 (préinstallé sur Windows 11)

```powershell
git clone https://github.com/phanoob/ScreenCraft-Windows.git
cd ScreenCraft-Windows

.\scripts\build-windows.ps1            # compile dist\ScreenCraft.exe
.\scripts\build-windows.ps1 -Install   # l'installe aussi, avec un raccourci dans le menu Démarrer
```

Aucune autorisation n'est à accorder sous Windows ; la caméra et le micro suivent les
interrupteurs de Paramètres › Confidentialité et sécurité. Le raccourci
d'enregistrement est **Ctrl+Shift+2**, les autres raccourcis utilisent Ctrl là où
macOS utilise ⌘, et l'application se trouve dans la zone de notification plutôt que
dans la barre de menus. Les réglages sont stockés dans `%APPDATA%\ScreenCraft` ; les
enregistrements et les exports vont dans `Vidéos\ScreenCraft`.

## Utilisation

1. Choisissez **Écran / Fenêtre / Zone**, les sources caméra et audio, puis appuyez
   sur **Enregistrer** (ou **⌘⇧2** / **Ctrl+Shift+2**). Appuyez sur **Arrêter** dans
   le bandeau ou refaites le raccourci.
2. L'enregistrement s'ouvre dans l'éditeur. La première ouverture prépare une copie
   d'aperçu fluide des images ; cela prend un peu de temps pour les longs
   enregistrements, puis elle est réutilisée.
3. Ajustez les zooms, l'apparence et le son ; ajoutez de la musique si vous voulez ;
   **⌘S** / **Ctrl+S** pour enregistrer.
4. **Exporter** (en haut à droite). Les fichiers vont par défaut dans `~/Movies` sur
   macOS et dans `Vidéos\ScreenCraft` sous Windows.

Les projets se trouvent dans `~/ScreenCraft Projects/<nom>.screencraft` sur macOS (dans
`Vidéos\ScreenCraft` sous Windows) et se renomment en cliquant sur le titre dans
l'éditeur.

### Raccourcis

`Espace` lecture / pause · `S` couper à la tête de lecture · `Z` ajouter un zoom ·
`D` dupliquer · `⌫` supprimer la sélection · `←` `→` image par image ·
`⌘Z` / `⇧⌘Z` annuler / rétablir · `⌘S` enregistrer · `⌘⇧2` démarrer / arrêter
l'enregistrement (sous Windows, Ctrl remplace ⌘ ; Ctrl+Y rétablit aussi)

## Comment c'est construit

| Élément | Emplacement | Rôle |
| --- | --- | --- |
| `screencraft-core` | `crates/core` | Rust pur : modèle de projet, physique de ressorts, correspondance de la timeline, génération des zooms automatiques et piste de rendu caméra / curseur image par image. Testé unitairement. |
| Application | `src-tauri` | Application Tauri 2 : capture ScreenCaptureKit, suivi des entrées et du curseur de texte, lecture / écriture des projets, proxy d'aperçu natif et serveur d'images, nivellement audio, compression AVFoundation, fenêtres et menus. |
| Interface | `ui` | Modules ES simples, sans bundler : barre d'enregistrement, éditeur, compositeur sur canevas et timeline. Export avec WebCodecs. Traductions dans `ui/js/locales` (voir `ui/js/i18n.js`). |
| Partie Windows | `src-tauri/src/win` | Remplace un par un les modules macOS : Windows Graphics Capture (`windows-capture`) + H.264/AAC Media Foundation pour l'enregistrement, WASAPI loopback pour le son système, hooks Win32 et UI Automation pour les entrées et le curseur de texte, décodage Media Foundation pour les aperçus, les images d'export et la compression. |

Quelques détails utiles :

- **Proxy d'aperçu.** L'éditeur ne lit jamais d'élément `<video>`. Rust décode
  l'enregistrement une seule fois en une petite bande d'images JPEG à côté du projet,
  et sert les images à la demande via un protocole `sc://` personnalisé ; le son est
  décodé et programmé avec WebAudio. Cela évite un blocage du lecteur WebKit /
  AVFoundation et rend le défilement instantané. En pause, les images sont rechargées
  en pleine résolution.
- **L'export** lit nativement les images sources en pleine résolution, les dessine
  avec le même code de composition que l'aperçu, et les encode avec WebCodecs et un
  multiplexeur MP4 intégré (`ui/vendor`, MIT). Aucun ffmpeg n'est nécessaire.
- **Les fonds d'écran** sont générés de façon procédurale ; aucune image tierce n'est
  incluse.

Un paquet `.screencraft` contient `screen.mp4`, éventuellement `camera.mp4` et
`mic.m4a`, `input.json` (curseur, clics, touches, curseur de texte), `project.json`, et
des fichiers de cache générés `proxy-*` / `mic-leveled-*` que l'on peut supprimer sans
risque.

## Développement

```bash
cargo test -p screencraft-core     # tests unitaires du module core
cargo check -p screencraft         # vérification des types de l'application
./scripts/bundle.sh --install      # recompiler et réinstaller (macOS)
```

Sous Windows, `.\scripts\build-windows.ps1 -Install` recompile et réinstalle.

## Licence

MIT, voir [LICENSE](LICENSE).

## Limitations connues

- macOS et Windows uniquement (pas de version Linux).
- Sous Windows, l'encodeur H.264 limite les enregistrements à 4096 pixels sur le grand
  côté, et le son système ne peut pas exclure les sons de ScreenCraft lui-même (macOS
  le peut).
- Les proxys d'aperçu sont volumineux (environ 2 à 3 Mo par seconde d'enregistrement) ;
  supprimez les fichiers `proxy-*` d'un dossier de projet pour libérer de la place.
- L'export n'est pas en temps réel ; les longs projets en 60 i/s prennent du temps.
- Le suivi de la frappe dépend de l'application active, qui doit exposer son curseur
  de texte via l'accessibilité (UI Automation sous Windows) ; sinon, sa position est
  estimée à partir des frappes.
- Pas de signature de code ni de notarisation : vous compilez l'application vous-même.
