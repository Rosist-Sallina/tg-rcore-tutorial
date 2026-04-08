// doomgeneric_rcore.c — rCore platform backend for doomgeneric
//
// Implements the 6 platform functions required by doomgeneric.h,
// plus main(). Uses rCore syscalls for framebuffer, timer, keyboard.

#include "doomkeys.h"
#include "doomgeneric.h"
#include "rcore_syscall.h"

#include <string.h>
#include <stdint.h>
#include <ctype.h>

// ===== Framebuffer state =====
static int fb_fd = -1;
static uint32_t *fb_ptr = NULL;
static unsigned int screen_w = 0;
static unsigned int screen_h = 0;

// ===== Keyboard state =====
static int kb_fd = -1;

#define KEYQUEUE_SIZE 16
static unsigned short s_KeyQueue[KEYQUEUE_SIZE];
static unsigned int s_KeyQueueWriteIndex = 0;
static unsigned int s_KeyQueueReadIndex = 0;

// ===== Linux input event key codes -> Doom key mapping =====
// (key codes match the values from breakout.rs / Linux input.h)
#define LINUX_KEY_ESC       1
#define LINUX_KEY_1         2
#define LINUX_KEY_2         3
#define LINUX_KEY_3         4
#define LINUX_KEY_4         5
#define LINUX_KEY_5         6
#define LINUX_KEY_6         7
#define LINUX_KEY_7         8
#define LINUX_KEY_8         9
#define LINUX_KEY_9         10
#define LINUX_KEY_0         11
#define LINUX_KEY_MINUS     12
#define LINUX_KEY_EQUAL     13
#define LINUX_KEY_BACKSPACE 14
#define LINUX_KEY_TAB       15
#define LINUX_KEY_Q         16
#define LINUX_KEY_W         17
#define LINUX_KEY_E         18
#define LINUX_KEY_R         19
#define LINUX_KEY_T         20
#define LINUX_KEY_Y         21
#define LINUX_KEY_U         22
#define LINUX_KEY_I         23
#define LINUX_KEY_O         24
#define LINUX_KEY_P         25
#define LINUX_KEY_A         30
#define LINUX_KEY_S         31
#define LINUX_KEY_D         32
#define LINUX_KEY_F         33
#define LINUX_KEY_G         34
#define LINUX_KEY_H         35
#define LINUX_KEY_J         36
#define LINUX_KEY_K         37
#define LINUX_KEY_L         38
#define LINUX_KEY_Z         44
#define LINUX_KEY_X         45
#define LINUX_KEY_C         46
#define LINUX_KEY_V         47
#define LINUX_KEY_B         48
#define LINUX_KEY_N         49
#define LINUX_KEY_M         50
#define LINUX_KEY_ENTER     28
#define LINUX_KEY_LCTRL     29
#define LINUX_KEY_LSHIFT    42
#define LINUX_KEY_RSHIFT    54
#define LINUX_KEY_LALT      56
#define LINUX_KEY_SPACE     57
#define LINUX_KEY_RCTRL     97
#define LINUX_KEY_UP        103
#define LINUX_KEY_LEFT      105
#define LINUX_KEY_RIGHT     106
#define LINUX_KEY_DOWN      108

// Map from Linux key code to a printable/Doom key
static unsigned char linuxKeyToDoom(uint16_t code)
{
    switch (code) {
    case LINUX_KEY_ENTER:    return KEY_ENTER;
    case LINUX_KEY_ESC:      return KEY_ESCAPE;
    case LINUX_KEY_LEFT:     return KEY_LEFTARROW;
    case LINUX_KEY_RIGHT:    return KEY_RIGHTARROW;
    case LINUX_KEY_UP:       return KEY_UPARROW;
    case LINUX_KEY_DOWN:     return KEY_DOWNARROW;
    case LINUX_KEY_LCTRL:
    case LINUX_KEY_RCTRL:    return KEY_FIRE;
    case LINUX_KEY_SPACE:    return KEY_USE;
    case LINUX_KEY_LSHIFT:
    case LINUX_KEY_RSHIFT:   return KEY_RSHIFT;
    case LINUX_KEY_LALT:     return KEY_LALT;
    case LINUX_KEY_TAB:      return KEY_TAB;
    // Letter keys -> lowercase ASCII
    case LINUX_KEY_A: return 'a';
    case LINUX_KEY_B: return 'b';
    case LINUX_KEY_C: return 'c';
    case LINUX_KEY_D: return 'd';
    case LINUX_KEY_E: return 'e';
    case LINUX_KEY_F: return 'f';
    case LINUX_KEY_G: return 'g';
    case LINUX_KEY_H: return 'h';
    case LINUX_KEY_I: return 'i';
    case LINUX_KEY_J: return 'j';
    case LINUX_KEY_K: return 'k';
    case LINUX_KEY_L: return 'l';
    case LINUX_KEY_M: return 'm';
    case LINUX_KEY_N: return 'n';
    case LINUX_KEY_O: return 'o';
    case LINUX_KEY_P: return 'p';
    case LINUX_KEY_Q: return 'q';
    case LINUX_KEY_R: return 'r';
    case LINUX_KEY_S: return 's';
    case LINUX_KEY_T: return 't';
    case LINUX_KEY_U: return 'u';
    case LINUX_KEY_V: return 'v';
    case LINUX_KEY_W: return 'w';
    case LINUX_KEY_X: return 'x';
    case LINUX_KEY_Y: return 'y';
    case LINUX_KEY_Z: return 'z';
    // Number keys
    case LINUX_KEY_0: return '0';
    case LINUX_KEY_1: return '1';
    case LINUX_KEY_2: return '2';
    case LINUX_KEY_3: return '3';
    case LINUX_KEY_4: return '4';
    case LINUX_KEY_5: return '5';
    case LINUX_KEY_6: return '6';
    case LINUX_KEY_7: return '7';
    case LINUX_KEY_8: return '8';
    case LINUX_KEY_9: return '9';
    case LINUX_KEY_MINUS:    return KEY_MINUS;
    case LINUX_KEY_EQUAL:    return KEY_EQUALS;
    default:                 return 0;
    }
}

static void addKeyToQueue(int pressed, unsigned char key)
{
    unsigned short keyData = (unsigned short)((pressed << 8) | key);
    s_KeyQueue[s_KeyQueueWriteIndex] = keyData;
    s_KeyQueueWriteIndex++;
    s_KeyQueueWriteIndex %= KEYQUEUE_SIZE;
}

// ===== Platform functions =====

void DG_Init()
{
    memset(s_KeyQueue, 0, sizeof(s_KeyQueue));

    // Open framebuffer
    fb_fd = rcore_open("/dev/fb0", RCORE_O_RDWR);
    if (fb_fd < 0) return;

    // Get resolution
    unsigned int res[2] = {0, 0};
    rcore_ioctl(fb_fd, RCORE_FB_GET_RESOLUTION, (unsigned long)res);
    screen_w = res[0];
    screen_h = res[1];

    // Map framebuffer into memory
    long fb_base = rcore_mmap(0,
                              screen_w * screen_h * 4,
                              RCORE_PROT_READ | RCORE_PROT_WRITE,
                              RCORE_MAP_PRIVATE,
                              fb_fd, 0);
    if (fb_base <= 0) return;
    fb_ptr = (uint32_t *)fb_base;

    // Open keyboard
    kb_fd = rcore_open("/dev/input0", RCORE_O_RDWR);
}

static int frame_count = 0;
void DG_DrawFrame()
{
    frame_count++;
    // Blit DG_ScreenBuffer to framebuffer.
    // Doom renders at DOOMGENERIC_RESX x DOOMGENERIC_RESY (640x400).
    // If screen size matches, direct copy. Otherwise, center or crop.
    if (fb_ptr != NULL) {
        unsigned int doom_w = DOOMGENERIC_RESX;
        unsigned int doom_h = DOOMGENERIC_RESY;

        // Center the Doom frame on screen
        unsigned int off_x = (screen_w > doom_w) ? (screen_w - doom_w) / 2 : 0;
        unsigned int off_y = (screen_h > doom_h) ? (screen_h - doom_h) / 2 : 0;
        unsigned int copy_w = (doom_w < screen_w) ? doom_w : screen_w;
        unsigned int copy_h = (doom_h < screen_h) ? doom_h : screen_h;

        for (unsigned int y = 0; y < copy_h; y++) {
            memcpy(&fb_ptr[(y + off_y) * screen_w + off_x],
                   &DG_ScreenBuffer[y * doom_w],
                   copy_w * sizeof(uint32_t));
        }

        // Flush framebuffer
        rcore_ioctl(fb_fd, RCORE_FB_FLUSH, 0);
    }

    // Poll keyboard input
    if (kb_fd >= 0) {
        struct rcore_input_event event;
        while (1) {
            long ret = rcore_read(kb_fd, &event, sizeof(event));
            if (ret <= 0) break;

            if (event.event_type == RCORE_EV_KEY) {
                unsigned char doom_key = linuxKeyToDoom(event.code);
                if (doom_key != 0) {
                    // value=1: press, value=0: release, value=2: repeat (treat as press)
                    int pressed = (event.value != 0) ? 1 : 0;
                    addKeyToQueue(pressed, doom_key);
                }
            }
        }
    }
}

void DG_SleepMs(uint32_t ms)
{
    // struct rcore_timespec ts;
    // ts.tv_sec = ms / 1000;
    // ts.tv_nsec = (ms % 1000) * 1000000UL;
    // rcore_nanosleep(&ts);
    uint32_t start_time = DG_GetTicksMs();
    while (DG_GetTicksMs() - start_time < ms) {
        rcore_sched_yield();
    }
}

uint32_t DG_GetTicksMs()
{
    struct rcore_timespec ts;
    rcore_clock_gettime(0, &ts); // CLOCK_REALTIME = 0
    return (uint32_t)(ts.tv_sec * 1000 + ts.tv_nsec / 1000000);
}

int DG_GetKey(int* pressed, unsigned char* doomKey)
{
    if (s_KeyQueueReadIndex == s_KeyQueueWriteIndex)
    {
        return 0;
    }
    else
    {
        unsigned short keyData = s_KeyQueue[s_KeyQueueReadIndex];
        s_KeyQueueReadIndex++;
        s_KeyQueueReadIndex %= KEYQUEUE_SIZE;

        *pressed = keyData >> 8;
        *doomKey = keyData & 0xFF;

        return 1;
    }
}

void DG_SetWindowTitle(const char * title)
{
    (void)title;
}

// ===== Entry point =====

int main(int argc, char **argv)
{
    doomgeneric_Create(argc, argv);

    while (1)
    {
        doomgeneric_Tick();
    }

    return 0;
}
