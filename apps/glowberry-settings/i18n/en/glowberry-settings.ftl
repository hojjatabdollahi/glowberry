# GlowBerry Settings - English translations

app-title = GlowBerry
app-description = Wallpaper settings for COSMIC desktop

# Library
filter-all = All
filter-images = Images
filter-live = Live
filter-colors = Colors
search-library = Search
library-empty = Nothing matches your search.
no-shaders = No shaders found. Install shaders to ~/.local/share/glowberry/shaders/
add-images = Add images
add-folder = Add folder
wp-remove-source = Remove from library
color-gradient = Gradient
solid-color = Solid color
gpu-pill = { $load } GPU
badge-all = all
badge-some = { $n } of { $total }

# Canvas
select-all-displays = Select all displays
tip-layer-up = Move layer up
tip-layer-down = Move layer down
tip-center = Center layer on the displays
tip-delete = Delete layer
tip-clear-all = Remove everything from the canvas
tip-fit = Fit all displays in view
ctx-bring-forward = Bring Forward
ctx-send-back = Send Back
ctx-remove = Remove

# Inspector
all-displays = All Displays
n-displays = { $n } displays
no-displays = No displays found.
mixed-content = These displays show different things. Pick from the library to set them all at once, or choose one.
nothing-staged = Nothing here yet. Pick something from the library.
nothing-yet = Nothing yet
spanning = One image across { $displays }
spanned-image = { $name } (spanned)
placement = Placement
placement-each = Duplicate
placement-span = Span
fit = Fit
fit-zoom = Zoom to fill
fit-inside = Fit inside
fit-stretch = Stretch
adapted-by = Adapted by { $author }
gpu-load = GPU load
resource-low = Low
resource-medium = Medium
resource-high = High
frame-rate = Frame rate
fps-15 = 15 FPS
fps-30 = 30 FPS
fps-60 = 60 FPS
render-quality = Render quality
quality-full = Full
quality-half = Half (best efficiency)
quality-quarter = Quarter
reset-to-defaults = Reset to defaults
shader-source = Source
shader-license = License

# Footer
hint-empty = Pick something from the library to put it on your displays.
status-applied = Everything applied
status-changed = { $n ->
    [one] 1 display changed, not applied yet
   *[other] { $n } displays changed, not applied yet
}
apply = Apply
revert = Revert

# Settings drawer
settings = Settings
background-service = Background Service
use-glowberry = Use GlowBerry as default
path-order-warning = Warning: ~/.local/bin must come before /usr/bin in PATH for this to work
appearance = Appearance
window-opacity = Window Opacity
prefer-low-power = Prefer low power GPU
power-saving = Power Saving
on-battery = On battery power
action-nothing = Do nothing
action-pause = Pause animation
action-reduce-15 = Reduce to 15 FPS
action-reduce-10 = Reduce to 10 FPS
action-reduce-5 = Reduce to 5 FPS
pause-low-battery = Pause on low battery
low-battery-threshold = Battery threshold
pause-lid-closed = Pause when lid closed

# About
repository = Repository
about = About

# Help dock
tip-1 = Click a display to choose it. Ctrl-click to choose several. Click the empty canvas to choose all of them.
tip-2 = Pick an image, a live wallpaper, or a color from the library to put it on the chosen displays.
tip-3 = With several displays chosen, Placement can span one image across them. Drag its corners on the canvas to resize it.
tip-4 = Nothing changes on your desktop until you press Apply. Revert throws the staged changes away.
tip-5 = Scroll on the canvas to zoom and drag the background to pan. The fit button brings everything back into view.
tip-6 = Right-click something on the canvas to bring it forward, send it back, or remove it.
tip-next = Next tip
tip-hide = Hide tips

# Context menus
put-on-all = Put on all displays
put-on = Put on { $display }
span-all = Span across all displays
ctx-select = Select
ctx-add-selection = Add to selection
ctx-duplicate-all = Duplicate on all displays
ctx-span-all = Span across all displays
ctx-clear-display = Clear display
ctx-fit = Fit to all displays
