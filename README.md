# Hollow

Jeu voxel Bevy. Lancer avec `cargo run --release`.

Le dépôt voisin `../minegen` est requis (dépendance Cargo locale).
Le terrain utilise `mc_worldgen::world::WorldTerrainGenerator` puis
`DecorationRegion` pour les lacs, géodes, minerais, sources et sélecteurs
arbres/bambous pris en charge. Aucune JVM n'est nécessaire. Les adaptations de
structures sont explicitement vides ; ce n'est pas un monde Minecraft FULL.

La décoration de chaque chunk source est calculée sur son terrain et huit voisins.
Ses modifications dans les neuf chunks sont conservées. Chaque chunk cible
combine les contributions de ses neuf sources en ordre X/Z fixe ; ses halos
sont synchronisés avec les chunks finaux chargés. Cette composition reste une adaptation de
jeu, pas une reproduction de l'ordonnanceur de décoration Vanilla : les features
de deux sources ne lisent pas leurs modifications mutuelles. L'ordre de
chargement et le thread ne changent pas cette composition.

Les définitions non compilables sont signalées une fois dans les logs. Si un
sélecteur d'arbres manque, une configuration d'arbre `minegen` prise en charge
est placée selon le biome avec une distribution propre au jeu. Une erreur
pendant le passage végétation abandonne la région partielle, recommence sans
végétation et utilise cette même solution de repli. Si le repli échoue aussi,
le terrain est reconstruit sans ces arbres. Une étape de terrain refusée est
signalée dans les logs et sa région partielle est abandonnée.

Les chunks font 16×16×384. Le stockage du jeu reste à Y positif :
`Y jeu = Y Minecraft + 64`, niveau marin à Y=127. La seed reste un `u32`.
La génération utilise deux workers dédiés pour laisser les tâches de mesh et de
rendu disponibles. Un cache partagé borné réutilise notamment le chunk calculé
au choix du point d'apparition. Les contextes de terrain restent propres à chaque
worker, avec leurs caches bornés.
Un chunk peut être affiché avant ses voisins : les bordures absentes sont
provisoirement de l'air, puis synchronisées dans les deux sens au chargement.
Les coordonnées négatives et les chunks modifiés suivent le même chemin. Le joueur attend le collider du chunk avant de tomber.

## Textures et crédits

Les textures du pack **Faithful 32x Java** utilisées sont :
`grass_block_top`, `grass_block_side`, `dirt`, `stone`, `sand`, `snow`,
`oak_log`, `oak_log_top`, `oak_leaves`, `deepslate`, `gravel`,
`coal_ore`, `iron_ore`, `copper_ore`, `gold_ore`, `redstone_ore`, `diamond_ore`,
`emerald_ore`, `lapis_ore`.

Création : [Faithful Resource Pack](https://faithfulpack.net).
Source : [Faithful-32x-Java](https://github.com/Faithful-Resource-Pack/Faithful-32x-Java).
Licence conservée dans `assets/voxel/packs/faithful/LICENSE.txt`.
L'array PNG est une adaptation : empilement des 19 textures et fermeture des
trous des feuilles, car le rendu actuel utilise des blocs opaques.
La révision et les SHA-256 des fichiers source sont dans `source.json`.

Pour régénérer les ressources : `python3 -m pip install Pillow`, puis
`python3 scripts/import_faithful.py`. Aucun téléchargement au lancement du jeu.
Faithful est le pack par défaut ; les anciens réglages enregistrés conservent
leur sélection. Choisir **Faithful 32x** dans les paramètres pour les remplacer.

## Eau et apparition

Le joueur cherche désormais un sol sec et deux cellules libres près de l'origine.
Pour la seed 42, l'ancien point fixe était submergé (surface Minecraft Y=63,
fond Y=26). Les aquifères de `minegen` sont conservés : les grottes peuvent donc
contenir de l'eau. Aucun remplissage global ni drainage artificiel n'est ajouté.
Le mesh d'eau dessine toutes les interfaces avec l'air, y compris les côtés et
le dessous ; aucune surface n'est émise dans un plafond solide. Les normales des
côtés sont conservées dans le shader et les vaguelettes concernent le dessus.

## Limites actuelles

- Décoration partielle : pas de structures, pas de simulation des ticks de
  fluides/feuilles, ni de petites plantes en géométrie croisée. Les plantes non
  solides non représentées sont omises, au lieu de devenir des cubes de pierre.
- 17 blocs solides sont représentés. Les huit minerais, la deepslate et le
  gravier ont leurs textures Faithful ; les packs historiques utilisent leur
  pierre comme solution de repli. Les variantes de minerai dans la deepslate
  partagent la texture du minerai normal.
- Tous les bois/feuillages partagent encore le chêne et des teintes constantes ;
  les orientations de troncs ne sont pas conservées. Les matériaux solides non
  représentés restent en pierre, y compris la lave. Glace et neige partagent
  la texture neige.
- La décoration et les colliders sur 384 couches coûtent davantage que l'ancien
  terrain. Les caches sont bornés par thread ; utiliser le profil release.
- L'éclairage souterrain et l'occlusion ambiante restent à améliorer.

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
