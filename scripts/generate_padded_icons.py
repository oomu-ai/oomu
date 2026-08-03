import sys
import os
from pathlib import Path
from PIL import Image


def pad_icon(source_path, dest_path):
    print(f"Padding {source_path} -> {dest_path}")
    try:
        # Open source image
        im = Image.open(source_path)

        # Ensure RGBA mode
        if im.mode != "RGBA":
            im = im.convert("RGBA")

        # Resize to 824x824 using high-quality lanczos
        im_resized = im.resize((824, 824), Image.Resampling.LANCZOS)

        # Create a new 1024x1024 transparent canvas
        canvas = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))

        # Center-paste the resized image (offset x=100, y=100)
        canvas.paste(im_resized, (100, 100), im_resized)

        # Save to destination
        dest_path.parent.mkdir(parents=True, exist_ok=True)
        canvas.save(dest_path, "PNG")
        print("Success.")
    except Exception as e:
        print(f"Error padding icon: {e}")
        sys.exit(1)


def icon_source_root():
    if len(sys.argv) > 1:
        return Path(sys.argv[1]).expanduser()
    configured = os.environ.get("OOMU_ICON_SOURCE_DIR")
    if configured:
        return Path(configured).expanduser()
    print(
        "Usage: python scripts/generate_padded_icons.py <source-dir>\n"
        "or set OOMU_ICON_SOURCE_DIR to the directory containing the exported OOMU icon PNGs."
    )
    sys.exit(2)


if __name__ == "__main__":
    repo_root = Path(__file__).resolve().parents[1]
    source_root = icon_source_root()

    # Light Mode Padded Icon (Correct Blue Default Dock Icon)
    pad_icon(
        source_root / "OOMU-macOS-Default-1024x1024@1x.png",
        repo_root / "src-tauri/icons/OOMU-macOS-Default-1024x1024@1x.png"
    )

    # Dark Mode Padded Icon (Correct Dark Default Dock Icon)
    pad_icon(
        source_root / "OOMU-macOS-Dark-1024x1024@1x.png",
        repo_root / "src-tauri/icons/OOMU-macOS-Dark-1024x1024@1x.png"
    )
