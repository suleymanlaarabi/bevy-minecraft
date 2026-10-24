- Ne jamais compiler le projet en mode **release**.
- Pour vérifier le travail, utiliser uniquement `cargo check`. Ne pas lancer le projet.
- Pour les optimisations de performances, ne pas chercher à les valider avec des benchmarks ou des tests. L'inspection du code et le raisonnement suffisent.
- Toujours exploiter au mieux **Bevy** et ses fonctionnalités modernes.
- Privilégier autant que possible les principes de l'**ECS**, le **parallélisme** et le **data-oriented design**.
- Les performance sont primordiale

## Systèmes Bevy et conflits de requêtes

- Avant d'ajouter ou de modifier un système, vérifier les accès de **tous** ses paramètres `Query` et ressources. Deux requêtes susceptibles de viser les mêmes entités ne doivent pas lire et modifier simultanément le même composant (`B0001`).
- Les filtres `Changed<T>` et `Added<T>` accèdent aussi au composant `T` : une requête filtrée ainsi peut entrer en conflit avec une autre requête contenant `&mut T`, même si `T` ne figure pas dans les données de la première.
- Privilégier une requête unique et `Mut<T>::is_changed()` / `is_added()` pour détecter les changements. Si plusieurs requêtes sont nécessaires, utiliser un `ParamSet` avec accès successifs ; utiliser `With` / `Without` uniquement si les ensembles d'entités sont réellement disjoints.
- `cargo check` ne détecte pas les conflits d'accès ECS : relire explicitement les paramètres de chaque système concerné, sans lancer le jeu pour contourner les règles de vérification ci-dessus.
