# MTGA Collection Extractor

Extract your owned cards from **MTG Arena** and export your collection to **CSV**, **JSON**, or **TXT**.

## Credits 
This project is an improved version of the original tool by NthPhantom10:  
[MTGA Collection Exporter](https://github.com/NthPhantom10/MTGA-collection-exporter)

## Improvements
- It just works :)
- Improved memory scanning performance
- Does not requires Scryfall database, reads local MTGA files.
- Various QOL features (better ui, auto loolup for entered anchors, auto path detection, etc) 

## Usage

1. Launch **MTG Arena**.
2. Open the **Collection** screen (**Decks** tab -> **Collection**)
3. Scroll through your collection for 5–10 seconds so the game loads card data into memory.
4. Run this tool and follow the on-screen prompts.
5. When prompted to enter cards, choose cards that you owning with thier quantities. Choose cards that are unlikely to appear together in any deck to reduce potential collisions. 
6. Wait for the scan to complete. Depending on your system, this may take up to 5 min.

## Notes

### Accuracy

- Works best on **large collections** (1000+) — error rate rough up to 1-3-5%.
- **Small collections** may show higher error rates.

### Reliability
- Keep MTGA Arena open while scanning.
- For best results, stay on the Collection screen during extraction.
- If detection quality is low, repeat the process and scroll through more pages before scanning, or enter different cards. 
- Restart the game if results seem off — the garbage collector may free stale memory blocks over time.
- In some cases you may need to scroll the entire collection in-game for the best results.


## Issues

If you have any issues, feel free to create issue :)
