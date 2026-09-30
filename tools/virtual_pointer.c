/*
 * virtual_pointer.c — a REAL pointer for a headless compositor.
 *
 * A headless sway has no input devices, so its seat advertises no pointer
 * capability and no client ever binds `wl_pointer`: `swaymsg seat … cursor`
 * moves a cursor nothing is listening to. This client creates a pointer
 * DEVICE through wlroots' `zwlr_virtual_pointer_manager_v1` — the protocol
 * sway implements for exactly this — so the seat gains the capability and
 * every event below reaches the surface under the cursor through the
 * compositor's own routing, as a mouse's would. It is the pointer twin of
 * `wtype` (PLAT-38's real key).
 *
 * Commands, one per line on stdin:
 *
 *   abs X Y        move to output position (X, Y) — absolute, in pixels
 *   down           press the left button
 *   up             release it
 *   wheel N        turn the wheel N notches (positive: down, toward the
 *                  user) — each a discrete step of 15, the value a mouse
 *                  wheel's click carries. A WHEEL, not a touchpad: GPUI
 *                  (gpui-pre-linux's `wl_pointer` handler) ignores a
 *                  continuous `axis` whose source is the wheel and scrolls
 *                  by the `axis_discrete` / `axis_value120` steps alone.
 *   sleep MS       wait MS milliseconds
 *   quit           exit (end of input does too)
 *
 * Usage: virtual_pointer <output-width> <output-height>   (the extent the
 * absolute positions are measured against: the headless output's size).
 *
 * Built by `scripts/build-virtual-pointer.sh` against the wlr-protocols XML.
 */
#define _POSIX_C_SOURCE 200809L
#include <linux/input-event-codes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <wayland-client.h>

#include "wlr-virtual-pointer-unstable-v1-client-protocol.h"

static struct zwlr_virtual_pointer_manager_v1 *manager;
static struct wl_seat *seat;

static void global_add(void *data, struct wl_registry *registry, uint32_t name,
                       const char *interface, uint32_t version) {
  (void)data;
  (void)version;
  if (strcmp(interface, zwlr_virtual_pointer_manager_v1_interface.name) == 0) {
    manager = wl_registry_bind(registry, name,
                               &zwlr_virtual_pointer_manager_v1_interface, 1);
  } else if (strcmp(interface, wl_seat_interface.name) == 0 && !seat) {
    seat = wl_registry_bind(registry, name, &wl_seat_interface, 1);
  }
}

static void global_remove(void *data, struct wl_registry *registry,
                          uint32_t name) {
  (void)data;
  (void)registry;
  (void)name;
}

static const struct wl_registry_listener registry_listener = {
    .global = global_add, .global_remove = global_remove};

static uint32_t now_ms(void) {
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return (uint32_t)(ts.tv_sec * 1000 + ts.tv_nsec / 1000000);
}

int main(int argc, char **argv) {
  if (argc != 3) {
    fprintf(stderr, "usage: %s <output-width> <output-height>\n", argv[0]);
    return 2;
  }
  uint32_t extent_w = (uint32_t)atoi(argv[1]);
  uint32_t extent_h = (uint32_t)atoi(argv[2]);
  struct wl_display *display = wl_display_connect(NULL);
  if (!display) {
    fprintf(stderr, "virtual_pointer: cannot connect to the compositor\n");
    return 1;
  }
  struct wl_registry *registry = wl_display_get_registry(display);
  wl_registry_add_listener(registry, &registry_listener, NULL);
  wl_display_roundtrip(display);
  if (!manager) {
    fprintf(stderr, "virtual_pointer: the compositor offers no "
                    "zwlr_virtual_pointer_manager_v1\n");
    return 1;
  }
  struct zwlr_virtual_pointer_v1 *ptr =
      zwlr_virtual_pointer_manager_v1_create_virtual_pointer(manager, seat);
  wl_display_roundtrip(display);

  char line[256];
  while (fgets(line, sizeof line, stdin)) {
    double a = 0, b = 0;
    if (sscanf(line, "abs %lf %lf", &a, &b) == 2) {
      zwlr_virtual_pointer_v1_motion_absolute(ptr, now_ms(), (uint32_t)a,
                                              (uint32_t)b, extent_w, extent_h);
      zwlr_virtual_pointer_v1_frame(ptr);
    } else if (strncmp(line, "down", 4) == 0) {
      zwlr_virtual_pointer_v1_button(ptr, now_ms(), BTN_LEFT,
                                     WL_POINTER_BUTTON_STATE_PRESSED);
      zwlr_virtual_pointer_v1_frame(ptr);
    } else if (strncmp(line, "up", 2) == 0) {
      zwlr_virtual_pointer_v1_button(ptr, now_ms(), BTN_LEFT,
                                     WL_POINTER_BUTTON_STATE_RELEASED);
      zwlr_virtual_pointer_v1_frame(ptr);
    } else if (sscanf(line, "wheel %lf", &a) == 1) {
      int notches = (int)a;
      zwlr_virtual_pointer_v1_axis_source(ptr, WL_POINTER_AXIS_SOURCE_WHEEL);
      zwlr_virtual_pointer_v1_axis_discrete(
          ptr, now_ms(), WL_POINTER_AXIS_VERTICAL_SCROLL,
          wl_fixed_from_double(15.0 * notches), notches);
      zwlr_virtual_pointer_v1_frame(ptr);
    } else if (sscanf(line, "sleep %lf", &a) == 1) {
      wl_display_roundtrip(display);
      struct timespec ts = {(time_t)(a / 1000),
                            (long)((long)a % 1000) * 1000000L};
      nanosleep(&ts, NULL);
      continue;
    } else if (strncmp(line, "quit", 4) == 0) {
      break;
    }
    wl_display_roundtrip(display);
  }
  zwlr_virtual_pointer_v1_destroy(ptr);
  wl_display_roundtrip(display);
  wl_display_disconnect(display);
  return 0;
}
