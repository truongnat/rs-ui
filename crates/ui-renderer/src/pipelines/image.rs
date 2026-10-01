use super::create;

pub(crate) fn build(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::RenderPipeline {
    create(
        device,
        layout,
        shader,
        format,
        samples,
        "ui-image-pipeline",
        "fs_main",
    )
}
