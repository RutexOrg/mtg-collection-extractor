# MTGA Collection Extractor

Extract your owned cards from **MTG Arena** and export your collection to **CSV**, **JSON**, or **TXT**.

## Credits 
This project is an improved version of the original tool by NthPhantom10:  
[MTGA Collection Exporter](https://github.com/NthPhantom10/MTGA-collection-exporter)

## Improvements

- Fixed a startup crash.
- Improved memory scanning performance with multithreading.
- Slightly improved UI.

## Usage

1. Launch **MTG Arena**.
2. Open the **Collection** screen.
3. Scroll through your collection for 5–10 seconds so the game loads card data into memory. (At the 07.2026 state, looks like you dont even need to open collections menu or scoll, game looks to load everything on boot)
4. Run this tool and follow the on-screen prompts.
5. When prompted to enter cards, choose cards that are unlikely to appear together in any deck together to reduce matching collisions. 
6. Wait for the scan to complete. Depending on your system, this may take up to 5-10 min.

## Notes

- Keep MTG Arena open while scanning.
- For best results, avoid switching away from the Collection screen during extraction.
- If detection quality is low, repeat the process and scroll through more pages before running the scan or enter another cards names with quantities.
- Does not guarantee to be 100% exact and may miss some cards or be very inacurate.