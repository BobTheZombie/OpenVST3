use std::{env, path::PathBuf};

fn main() {
    let sdk = env::var("VST3_SDK_DIR").expect("Set VST3_SDK_DIR to your local vst3sdk path");
    println!("cargo:rerun-if-env-changed=VST3_SDK_DIR");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let wrapper_h = out.join("v3shim.h");
    let wrapper_cpp = out.join("v3shim.cpp");

    let header = r#"
#pragma once
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    char category[64];
    char name[128];
    uint8_t cid[16];
} v3_class_info;

// Types are opaque pointers coming from the plugin binary/SDK
typedef void* v3_factory;
typedef void* v3_component;
typedef void* v3_audio_processor;
typedef void* v3_funknown;

// Factory utilities
int  v3_factory_class_count(v3_factory f);
int  v3_factory_class_info(v3_factory f, int idx, v3_class_info* out_info);
int  v3_factory_create_audio_processor(v3_factory f, const uint8_t cid[16], v3_audio_processor* out_proc, v3_component* out_comp);

// Lifetime
int  v3_release(v3_funknown obj);

// Component
int  v3_component_initialize(v3_component c);
int  v3_component_set_active(v3_component c, int state);
int  v3_component_terminate(v3_component c);

// Processor
int  v3_audio_processor_setup(v3_audio_processor p, double sample_rate, int32_t max_block, int32_t in_channels, int32_t out_channels);
int  v3_audio_processor_set_active(v3_audio_processor p, int state);

// Process (float32, deinterleaved channel pointers)
int  v3_audio_processor_process_f32(v3_audio_processor p,
    const float** inputs, int32_t in_channels,
    float** outputs, int32_t out_channels,
    int32_t num_samples);

#ifdef __cplusplus
}
#endif
"#;
    std::fs::write(&wrapper_h, header).unwrap();

    let impl_cpp = format!(r#"
#include <cstring>
#include <vector>
#include <string>

// VST3 SDK headers
#include <pluginterfaces/base/ipluginbase.h>
#include <pluginterfaces/base/funknown.h>
#include <pluginterfaces/base/futils.h>
#include <pluginterfaces/vst/ivstcomponent.h>
#include <pluginterfaces/vst/ivstaudioprocessor.h>
#include <pluginterfaces/vst/ivstprocesscontext.h>
#include <pluginterfaces/vst/vsttypes.h>

#include "{h}"

using namespace Steinberg;
using namespace Steinberg::Vst;

static inline FUID fromBytes(const uint8_t b[16]) {{ return FUID(b[0],b[1],b[2],b[3], b[4],b[5],b[6],b[7], b[8],b[9],b[10],b[11], b[12],b[13],b[14],b[15]); }}

extern "C" int v3_factory_class_count(void* f) {{
    auto* fac = reinterpret_cast<IPluginFactory*>(f);
    if (!fac) return -1;
    int32 count = 0;
    if (auto* f2 = FUnknownPtr<IPluginFactory2>(fac)) {{
        count = f2->countClasses();
    }} else {{
        count = fac->countClasses();
    }}
    return (int)count;
}}

extern "C" int v3_factory_class_info(void* f, int idx, v3_class_info* out_info) {{
    auto* fac = reinterpret_cast<IPluginFactory*>(f);
    if (!fac || !out_info) return -1;
    PClassInfo info{{}};
    tresult r = fac->getClassInfo((int32)idx, &info);
    if (r != kResultOk) return -2;
    std::memset(out_info, 0, sizeof(*out_info));
    std::strncpy(out_info->category, info.category, sizeof(out_info->category)-1);
    std::strncpy(out_info->name, info.name, sizeof(out_info->name)-1);
    std::memcpy(out_info->cid, info.cid, 16);
    return 0;
}}

extern "C" int v3_factory_create_audio_processor(void* f, const uint8_t cid_b[16], void** out_proc, void** out_comp) {{
    if (!f || !cid_b || !out_proc || !out_comp) return -1;
    auto* fac = reinterpret_cast<IPluginFactory*>(f);
    FUID cid = fromBytes(cid_b);

    FUnknown* unk = nullptr;
    tresult r = fac->createInstance(cid, Vst::IAudioProcessor::iid, (void**)&unk);
    if (r != kResultOk || !unk) return -2;
    auto* proc = FUnknownPtr<Vst::IAudioProcessor>(unk);
    auto* comp = FUnknownPtr<Vst::IComponent>(unk);
    if (!proc || !comp) {{ if (unk) unk->release(); return -3; }}

    *out_proc = proc.getInterface();
    *out_comp = comp.getInterface();
    return 0;
}}

extern "C" int v3_release(void* o) {{
    if (!o) return -1;
    auto* u = reinterpret_cast<FUnknown*>(o);
    return (int)u->release();
}}

extern "C" int v3_component_initialize(void* c) {{
    if (!c) return -1;
    auto* comp = reinterpret_cast<IComponent*>(c);
    return comp->initialize(nullptr) == kResultOk ? 0 : -2;
}}

extern "C" int v3_component_set_active(void* c, int state) {{
    if (!c) return -1;
    auto* comp = reinterpret_cast<IComponent*>(c);
    return comp->setActive(state ? true : false) == kResultOk ? 0 : -2;
}}

extern "C" int v3_component_terminate(void* c) {{
    if (!c) return -1;
    auto* comp = reinterpret_cast<IComponent*>(c);
    return comp->terminate() == kResultOk ? 0 : -2;
}}

extern "C" int v3_audio_processor_setup(void* p, double sample_rate, int32 max_block, int32 in_channels, int32 out_channels) {{
    if (!p) return -1;
    auto* proc = reinterpret_cast<IAudioProcessor*>(p);
    ProcessSetup setup{{}};
    setup.processMode = kRealtime;
    setup.symbolicSampleSize = kSample32;
    setup.maxSamplesPerBlock = max_block;
    setup.sampleRate = sample_rate;
    if (proc->setupProcessing(setup) != kResultOk) return -2;

    auto* comp = FUnknownPtr<IComponent>(proc);
    if (comp) {{
        // Try to activate main input/output buses
        comp->setActive(true);
    }}
    return 0;
}}

extern "C" int v3_audio_processor_set_active(void* p, int state) {{
    if (!p) return -1;
    auto* proc = reinterpret_cast<IAudioProcessor*>(p);
    return proc->setActive(state?true:false) == kResultOk ? 0 : -2;
}}

extern "C" int v3_audio_processor_process_f32(void* p,
    const float** inputs, int32 in_channels,
    float** outputs, int32 out_channels,
    int32 num_samples) {{
    if (!p) return -1;
    auto* proc = reinterpret_cast<IAudioProcessor*>(p);

    AudioBusBuffers inBuf{{}};
    AudioBusBuffers outBuf{{}};

    inBuf.numChannels = in_channels;
    inBuf.channelBuffers32 = const_cast<float**>(inputs);
    outBuf.numChannels = out_channels;
    outBuf.channelBuffers32 = outputs;

    AudioBusBuffers inputsArr[1] = {{ inBuf }};
    AudioBusBuffers outputsArr[1] = {{ outBuf }};

    ProcessData data{{}};
    data.numSamples = num_samples;
    data.numInputs =  in_channels > 0 ? 1 : 0;
    data.numOutputs = out_channels > 0 ? 1 : 0;
    data.inputs  = in_channels > 0 ? inputsArr : nullptr;
    data.outputs = out_channels > 0 ? outputsArr : nullptr;
    data.processMode = kRealtime;
    data.symbolicSampleSize = kSample32;

    return proc->process(data) == kResultOk ? 0 : -2;
}}

"#, h=wrapper_h.file_name().unwrap().to_string_lossy());
    std::fs::write(&wrapper_cpp, impl_cpp).unwrap();

    let mut build = cc::Build::new();
    build.cpp(true)
        .files([wrapper_cpp])
        .flag_if_supported("-std=c++17")
        .include(&sdk)
        .include(format!("{}/pluginterfaces", &sdk));

    // Note: on some distros you may need to link to stdc++ explicitly when consumed.
    build.compile("openvst3_shim");
}
