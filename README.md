# Hollow

Jeu voxel Bevy. Lancer avec `cargo run --release`.

Le dépôt voisin `../worldgen` (`pumpkin-worldgen`, façade du générateur vanilla
de Pumpkin, Minecraft 26.3) est requis (dépendance Cargo locale). Chaque chunk
vient de `World::generate_chunk` : terrain, biomes, grottes, minerais, arbres et
structures inclus. Les états de blocs sont convertis en `VoxelKind` (≈100 blocs
cubiques) ; les blocs non cubiques (plantes, torches, barrières…) sont omis et
les solides non représentés restent en pierre.

Les chunks font 16×16×384. Le stockage du jeu reste à Y positif :
`Y jeu = Y Minecraft + 64`, niveau marin à Y=127. La seed reste un `u32`.
La génération utilise un pool dédié et un contexte Pumpkin partagé. Un cache
borné réutilise notamment le chunk calculé au choix du point d'apparition.
Un chunk peut être affiché avant ses voisins : les bordures absentes sont
provisoirement de l'air, puis synchronisées dans les deux sens au chargement.
Les coordonnées négatives et les chunks modifiés suivent le même chemin. Le joueur attend le collider du chunk avant de tomber.

## Textures

Le pack par défaut **Minecraft** utilise les textures officielles extraites du
client Minecraft 26.3 téléchargé depuis les serveurs de Mojang (SHA-1 vérifié).
Elles ne peuvent pas être redistribuées : `blocks_array.png` est ignoré par git
et doit être généré localement. Sans lui, le jeu se replie sur Faithful :

```bash
python3 -m pip install Pillow
python3 scripts/import_vanilla.py   # version optionnelle en argument, ex. 26.3
```

Le script écrit aussi `pack.ron` (une entrée par `VoxelKind::key`) et extrait les
textures de fissures officielles dans `assets/voxel/breaking/vanilla/` :
les dix étapes sont appliquées en multiplication sur le bloc pendant la casse,
quel que soit le pack de blocs choisi. Ces PNG sont ignorés par git ; sans eux,
les fissures utilisent le pack de repli fourni avec le jeu.

Les packs historiques (Faithful, Replete, Meadow, Twilight) n'ont que quelques blocs ; les
autres utilisent leur repli (`VoxelKind::fallback`, ex. bouleau → chêne → pierre).
Faithful : voir `assets/voxel/packs/faithful/LICENSE.txt` et
`scripts/import_faithful.py`.

## Eau et apparition

Le joueur cherche désormais un sol sec et deux cellules libres près de l'origine.
Les candidats sont sondés sur le terrain seul (`generate_terrain`, sans grottes
ni décorations), puis le chunk retenu est généré complètement et reste en cache.
Les blocs immergés (coraux, cornichons de mer, clôtures noyées…) laissent de l'eau.
Pour la seed 42, l'ancien point fixe était submergé (surface Minecraft Y=63,
fond Y=26). Les aquifères vanilla sont conservés : les grottes peuvent donc
contenir de l'eau. Aucun remplissage global ni drainage artificiel n'est ajouté.
Le mesh d'eau dessine toutes les interfaces avec l'air, y compris les côtés et
le dessous ; aucune surface n'est émise dans un plafond solide. Les normales des
côtés sont conservées dans le shader et les vaguelettes concernent le dessus.

## Éclairage

Les chunks conservent la lumière du ciel et des blocs calculée par Pumpkin.
Les shaders du terrain et de l'eau combinent ces niveaux interpolés avec le soleil
et les ombres de Bevy. Les endroits abrités s'assombrissent et les sources lumineuses
existantes éclairent leur environnement avec une teinte légèrement chaude.
L'éclairage est toujours actif ; aucune option graphique supplémentaire n'est ajoutée.

Les poses et destructions mettent à jour les états Minecraft d'un contexte séparé,
via un worker dédié. Les propriétés lumineuses modifiées déclenchent un recalcul
local (chunk concerné et voisins chargés), sans régénération du terrain. Les bordures,
y compris diagonales, suivent les mises à jour ; les meshes conservent les gradients
lors des fusions. Une modification de lumière seule ne reconstruit pas les collisions.
Les résultats en retard ne remplacent pas les éditions plus récentes. Les chunks
modifiés conservent leurs données et leurs éditions lumineuses lors du déchargement.

## Limites actuelles

- Rendu en cubes uniquement : plantes, barrières, torches, vitres… sont omises ;
  dalles, escaliers et murets deviennent des cubes de leur matériau. Les blocs
  sans `VoxelKind` prennent le plus proche (planches de cerisier → chêne, laine →
  terre cuite, briques de deepslate → deepslate) ; le reste devient de la pierre
  (journalisé en `debug`). Pas de ticks de fluides/feuilles.
- Teintes de biome fixes (plaines) ; orientations des troncs non conservées.
  La lave est un bloc opaque où l'on s'enfonce comme dans l'eau, sans dégâts.
- Pumpkin partage les dépendances entre chunks voisins pour générer les décorations
  aux frontières. Les premiers chunks nécessitent aussi ces voisins.
- L'occlusion ambiante reste à améliorer.

Vérification : `cargo check --all-targets --features dev`, `cargo test --lib`.
Le test GPU peut être lancé avec
`cargo test --lib terrain_and_water_gpu_pipelines_compile_with_and_without_shadows -- --ignored`.

Pour un contrôle du rendu hors fenêtre :

```bash
HOLLOW_BENCH_HEADLESS=1 HOLLOW_DIAGNOSTICS_AUTOPLAY=1 HOLLOW_VOXEL_BENCH=1 HOLLOW_SCREENSHOT_PATH=/tmp/hollow.png cargo run --features dev
```

Les régions affichent les assemblages terminés pendant le chargement, puis se
complètent à mesure que les chunks arrivent. Les assets sont recherchés près du
binaire puis dans le dépôt de compilation, ou via `BEVY_ASSET_ROOT`.
