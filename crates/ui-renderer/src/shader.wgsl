struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(position, 0.0, 1.0);
    output.uv = uv;
    output.color = color;
    return output;
}

@group(0) @binding(0) var image_texture: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let sampled = textureSample(image_texture, image_sampler, input.uv);
    let premultiplied_sample = vec4<f32>(sampled.rgb * sampled.a, sampled.a);
    return premultiplied_sample * input.color;
}

@fragment
fn fs_text(input: VertexOutput) -> @location(0) vec4<f32> {
    let coverage = textureSample(image_texture, image_sampler, input.uv).r;
    return input.color * coverage;
}
