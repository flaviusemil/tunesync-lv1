// wmsd_abi.h - eMotion LV1 control-surface driver ABI ("WMSD", interface version 6).
//
// Copied from Headlink's tools/surfaceprobe/wmsd_abi.h (branch spike/fader-surface,
// 85d906e, 2026-10-25), unchanged, so Tune Remote and Headlink stay independent. The
// plug-in parameter addressing Tune Remote uses (section 4) is in core/tune.h.
//
// Reverse-engineered from LV1 17.3.50 (Mac: MixerCoreSG + Mackie_Control + FaderBank_Control
// + SurfaceDriver_App's libSurfaceDriver_RxLib, all with symbols; Windows: the stripped
// MixerCoreSG.dll / *_Control.dll, checked against the same constants). See
// docs/LV1-SURFACE-DRIVER.md for how each offset was found.
//
// LV1 never sees a C++ object: it resolves 18 C exports by name and passes the driver's
// opaque controller pointer back as the first argument. Data crosses as packed structs
// (Waves builds with #pragma pack(1): every size below is odd for that reason), and the
// layouts are identical on Windows and macOS - none of them contains a `long`.
#pragma once
#include <cstdint>

namespace wmsd {

constexpr int kInterfaceVersion = 6;   // WCSurfaceDriverManager::LoadSurfaceDriverFromPath: cmp eax,6

#pragma pack(push, 1)

// ---- WMSD_GetAvailableControllerInfo(ControllerInfo*) -------------------------------------
// LV1 zeroes 0x602 bytes and writes the driver's own path at +0x202 before the call.
// `type` empty => LV1 loads the driver but lists nothing (that is how SurfaceDriver_App,
// LV1's own GUI, stays out of Setup -> Control Surfaces).
struct ControllerInfo {
    char    name[0x80];       // +0x000  Mackie: "Mackie"
    char    category[0x80];   // +0x080  Mackie: "Surface"
    char    type[0x80];       // +0x100  the key; prefs <ControllerType> and CreateControllerForType's arg 1
    char    extra[0x80];      // +0x180
    uint8_t flag200;          // +0x200  Mackie sets 1
    uint8_t flag201;          // +0x201
    char    path[0x400];      // +0x202  in: this DLL's path
};
static_assert(sizeof(ControllerInfo) == 0x602, "ControllerInfo");

// ---- the host callbacks (arg 3 of WMSD_CreateControllerForType) ---------------------------
// Each entry is MixerCoreSG's exported <Name>Wrapper(core, struct*), which tail-calls the
// WCMixerCore virtual of the same name. `core` is arg 2 of CreateControllerForType.
struct CoreCallbacks {
    int (*SetMixerControl)(void* core, const void* control);
    int (*GetMixerControl)(void* core, void* control);
    int (*SetMixerAssignments)(void* core, const void* batch);
    int (*GetMixerAssignments)(void* core, void* batch);
    int (*SetMixerParam)(void* core, const void* param);
    int (*GetMixerParam)(void* core, void* param);
    int (*SendMixerCommand)(void* core, void* command);
};

// ---- WSMixerParam: the 0x35-byte header of every param --------------------------------------
// GetMixerParam dispatches on `type` and writes the WHOLE struct for that type - there is
// no size field to check, so the caller must pass a buffer at least that large.
struct ParamHeader {
    uint32_t f00;      // 0
    uint32_t f04;      // 2
    uint32_t f08;      // 0
    uint64_t sender;   // +0x0c  copied into the meter-register control: identifies the registrant
    uint32_t f14, f18; // 0
    uint64_t f1c;      // 0
    uint32_t f24;      // 0
    uint32_t type;     // +0x28  WEMixerParamType
    uint32_t f2c;      // 0xFFFFFFFF
    uint32_t f30;      // 0
    uint8_t  f34;      // 1
};
static_assert(sizeof(ParamHeader) == 0x35, "ParamHeader");

inline void init_param(ParamHeader* p, uint32_t type) {
    *p = ParamHeader{};
    p->f04 = 2; p->type = type; p->f2c = 0xFFFFFFFFu; p->f34 = 1;
}

enum ParamType : uint32_t {
    kParamIOBankConfig        = 6015,
    kParamMeterInfoBatch      = 7034,   // pushed via WMSD_MixerParamWasChanged
    kParamRegisterVUs         = 7048,   // sent via SetMixerParam to (de)register meters
    kParamSoundGridInventory  = 9000,
    kParamSoundGridInventory2 = 9001,
};

// ---- meters ---------------------------------------------------------------------------------
// MeterInfoBatch (7034) = ParamHeader + a POINTER to LV1's own WSAllMetersInfo (~10 MB).
// WSAllMetersInfo: 512 track blocks x 42 meters, each meter a 0x1E4-byte WSMixerControl,
// then u32 base[26] = first block of each WETrackTypes (-1 = none).
constexpr uint32_t kMaxTracks      = 512;
constexpr uint32_t kMetersPerTrack = 42;     // WEVUMeters 0..41
constexpr uint32_t kMeterRecSize   = 0x1E4;
constexpr uint32_t kTrackBlockSize = kMetersPerTrack * kMeterRecSize;   // 0x4F68
constexpr uint32_t kTypeBaseOff    = kMaxTracks * kTrackBlockSize;      // 0x9ED000
constexpr uint32_t kNumTrackTypes  = 26;
static_assert(kTrackBlockSize == 0x4F68 && kTypeBaseOff == 0x9ED000, "meters layout");

// Offsets inside one meter record (a WSMixerControl):
constexpr uint32_t kRecSize       = 0x00;   // u32, 0x1E4
constexpr uint32_t kRecTrackType  = 0x2C;   // u32  WSTrackID.type
constexpr uint32_t kRecTrackIndex = 0x30;   // u32  WSTrackID.index
constexpr uint32_t kRecId1        = 0x34;   // u32  control-id detail (meter position)
constexpr uint32_t kRecId2        = 0x38;
constexpr uint32_t kRecId3        = 0x3C;
constexpr uint32_t kRecId4        = 0x40;
constexpr uint32_t kRecValue      = 0x48;   // double: the level, LINEAR amplitude (0 = silence; dBFS = 20*log10)
constexpr uint32_t kRecClip       = 0x1D8;  // u8: clip flag from the DSP read
constexpr uint32_t kRecEnabled    = 0x1DA;  // u8: registered - LV1 only reads (updates) meters with this set
constexpr uint32_t kRecLast       = 0x1DB;  // double: previous value (init -25104.0, a "never read" sentinel)
constexpr uint32_t kRecChanged    = 0x1E3;  // u8: value changed on the last read
// WEVUMeters (rig-confirmed control ids): 0..15 = input VU of channel 0..15 (0, -1, ch, 0xb);
// 16..31 = output VU (7, -1, ch, 0xd); 32..41 = dynamics and the rest. A mono strip uses 0,
// a stereo strip 0/1 (WCMapperOverview_App_MR::MapGetsFromCoreToSurfaceCInfos_Global_MetersBatch_Track).

// RegisterVUs (7048) = ParamHeader + WSAllRegisterVUs: 512x42 entries + u32 base[26].
struct RegisterEntry { uint32_t type, index, meter; uint8_t on; };
static_assert(sizeof(RegisterEntry) == 13, "RegisterEntry");
struct RegisterVUs {
    ParamHeader   hdr;
    RegisterEntry e[kMaxTracks * kMetersPerTrack];
    uint32_t      base[kNumTrackTypes];
};
static_assert(sizeof(RegisterVUs) == 0x4449D, "RegisterVUs");

// ---- assignments (the patch) ----------------------------------------------------------------
// The 7 fields are the columns of LV1's session-DB `routes` table (src_/dst_ prefixed), e.g.
// Wing channel 8 -> input 1:  src{10,0,-1,-1,8,-1,-1} -> dst{0,0,0,0,0,-1,0}.
struct ChainPos {            // WSChainPosition
    uint32_t size;           // 0x24
    uint32_t ver;            // 2
    uint32_t trackType;      // cluster_type: 0 = input channel, 10 = SoundGrid IO device, 11 = ...
    uint32_t trackIndex;     // cluster_type_index (type 10: the device's IO-bank track handle)
    uint32_t f[5];           // section_type, section_index, channel_index, control_id, asgn_type
};
enum { kPosSectionType = 0, kPosSectionIndex = 1, kPosChannel = 2, kPosControlId = 3, kPosAsgnType = 4 };
static_assert(sizeof(ChainPos) == 0x24, "ChainPos");

struct Assignment {          // WSMixerAssignment
    ChainPos src;            // +0x00
    ChainPos dst;            // +0x24
    uint32_t f48;
    uint8_t  f4c;
    uint64_t f4d;            // -1
    uint64_t f55;            // 0
    uint32_t f5d;
};
static_assert(sizeof(Assignment) == 0x61, "Assignment");

struct AssignmentsFilter {   // WSAssignmentsFilter; Reset() = everything wildcard
    Assignment a;
    uint32_t   f61;          // 1
    uint8_t    f65;
    uint32_t   f66;          // -1
};
static_assert(sizeof(AssignmentsFilter) == 0x6A, "AssignmentsFilter");

struct AssignmentsBatch {    // WSMixerAssignmentsBatch
    uint32_t          size;      // 0xB0
    uint32_t          ver;       // 1
    uint8_t           f08[0x22];
    AssignmentsFilter filter;    // +0x2A
    Assignment*       items;     // +0x94  owned by the CALLER (GetMixerAssignments fills it)
    uint32_t          count;     // +0x9C
    uint32_t          capacity;  // +0xA0
    uint32_t          fa4;       // -1
    uint64_t          fa8;       // -1
};
static_assert(sizeof(AssignmentsBatch) == 0xB0, "AssignmentsBatch");

inline void init_assignment(Assignment* a) {
    *a = Assignment{};
    a->src.size = a->dst.size = 0x24;
    a->src.ver  = a->dst.ver  = 2;
    uint32_t* s = &a->src.trackType; for (int i = 0; i < 7; ++i) s[i] = 0xFFFFFFFFu;
    uint32_t* d = &a->dst.trackType; for (int i = 0; i < 7; ++i) d[i] = 0xFFFFFFFFu;
    a->dst.f[0] = 0;          // +0x34
    a->dst.f[4] = 0;          // +0x44 (the u64 0 at +0x44 also clears f48)
    a->f4d = ~0ull;
}

inline void init_batch(AssignmentsBatch* b) {
    *b = AssignmentsBatch{};
    b->size = 0xB0; b->ver = 1;
    init_assignment(&b->filter.a);
    b->filter.f61 = 1; b->filter.f66 = 0xFFFFFFFFu;
    b->fa4 = 0xFFFFFFFFu; b->fa8 = ~0ull;
}

// ---- inventory -------------------------------------------------------------------------------
// SoundGridInventory (9000/9001) = ParamHeader + WSSGInventory (0x71A1F bytes):
//   u32 count; WSSGDevice dev[64]; u32 count2; WSSGDevice dev2[64]; ...
constexpr uint32_t kInvBodySize   = 0x71A1F;
constexpr uint32_t kSGDeviceSize  = 0x978;
constexpr uint32_t kInvList1      = 0x35;       // param offsets
constexpr uint32_t kInvList2      = 0x25E39;
// Offsets inside a WSSGDevice:
constexpr uint32_t kDevF0         = 0x00;
constexpr uint32_t kDevAsgnIndex  = 0x04;   // -1 = not assigned
constexpr uint32_t kDevBank       = 0x08;
constexpr uint32_t kDevName       = 0x14;   // char[0x41]
constexpr uint32_t kDevMac        = 0x87;   // char[0x12] "00:1c:d1:xx:xx:xx"
constexpr uint32_t kDevFlag219    = 0x219;
constexpr uint32_t kDevF8ED       = 0x8ED;
constexpr uint32_t kDevF8F1       = 0x8F1;

// IOBankConfig (6015) = ParamHeader + { u32 f0; u32 bankSize[8]; }.
// A device's track handle (the index of WSTrackID{10, handle}) is
//   handle = asgnIndex + sum(bankSize[0 .. bank-1])     (bank 0 or -1/-2: just asgnIndex)
struct IOBankConfig { ParamHeader hdr; uint32_t f0; uint32_t bankSize[8]; };
static_assert(sizeof(IOBankConfig) == 0x59, "IOBankConfig");

// ---- mixer controls (docs/SPIKE-FADER-SURFACE.md) --------------------------------------------
// WSMixerControl is what SetMixerControl / GetMixerControl take and what
// WMSD_ControllerDisplayUpdate hands every surface. 0x1D8 bytes; a meter record (0x1E4) is one of
// these plus the meter tail. Layout from Mackie_Control's WCMackieMapper::HandleFaderControlByType
// (builds one) and MixerCoreSG's WCControlTrack::ProcessMixerControl_Control (consumes one).
enum ControlType : uint32_t {          // +0x20, wvMixerNS::ToString(WEControlType)
    kCtlControl = 0, kCtlTrackCtrl = 1, kCtlChainPos = 2, kCtlMixMtxPos = 3, kCtlCtrlProps = 4,
    kCtlGraph = 5, kCtlMeter = 6, kCtlDisplay = 7, kCtlFloatingWindows = 8, kCtlMemObj = 9,
    kCtlChannels = 10, kCtlPreset = 11, kCtlEnblMeters = 12, kCtlAdjMeters = 13, kCtlRegister = 14,
    kCtlTouch = 15, kCtlRelease = 16,
};
enum SectionType : uint32_t {          // WSSectionControlID.section, sWESectionType names
    kSecInput = 0, kSecFilter = 1, kSecDynamics = 2, kSecEQ = 3, kSecPlugins = 4, kSecAuxSend = 5,
    kSecMixer = 6, kSecOutput = 7, kSecName = 8, kSecDisplay = 9,
};
enum OutputControlId : uint32_t {      // control ids of section Output (7)
    kOutGain = 0, kOutPan = 6, kOutSelect = 9, kOutCue = 10, kOutMute = 11, kOutVU = 13,
};
enum TrackType : uint32_t {            // WSTrackID.type, ToString(WSTrackID)
    kTrkCh = 0, kTrkGp = 1, kTrkFx = 2, kTrkMain = 3, kTrkCenter = 4, kTrkMono = 5, kTrkMtx = 6,
    kTrkCue = 7, kTrkTB = 8, kTrkDCA = 12,
};
struct ControlID {                     // wvMixerNS::WSControlID, 0x20
    uint32_t size;                     // 0x20
    uint32_t f04;                      // 0
    uint32_t trackType, trackIndex;    // WSTrackID
    uint32_t section, sectionIndex, channel, controlId;   // WSSectionControlID; -1 = n/a
};
static_assert(sizeof(ControlID) == 0x20, "ControlID");
struct MixerControl {                  // wvMixerNS::WSMixerControl
    uint32_t  size;                    // +0x000  0x1D8
    uint32_t  structType;              // +0x004  0 = control (SetMixerControl refuses anything else)
    uint32_t  flags;                   // +0x008
    uint64_t  sender;                  // +0x00C  set by Mackie only in do-by-group mode
    uint8_t   f14[0x0C];               // +0x014
    uint32_t  ctlType;                 // +0x020  ControlType
    ControlID id;                      // +0x024
    uint32_t  valueType;               // +0x044  0 = absolute real-world (the only kind Set accepts)
    double    value;                   // +0x048  real-world: dB for a gain (-144 = -inf), 0/1 for a switch
    char      text[0x180];             // +0x050  string value (section Name: the track name)
    uint32_t  grpType, grpIndex;       // +0x1D0  do-by-group track, -1/-1 = none
};
static_assert(sizeof(MixerControl) == 0x1D8, "MixerControl");

inline void init_control(MixerControl* c, uint32_t ctlType, uint32_t trackType, uint32_t trackIndex,
                         uint32_t section, uint32_t controlId) {
    *c = MixerControl{};
    c->size = 0x1D8; c->ctlType = ctlType;
    c->id = ControlID{0x20, 0, trackType, trackIndex, section, 0xFFFFFFFFu, 0xFFFFFFFFu, controlId};
    c->grpType = c->grpIndex = 0xFFFFFFFFu;
}

#pragma pack(pop)

}  // namespace wmsd
