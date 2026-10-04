#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Environment {
    FreeWorld,
    Windows,
    Linux,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageFormat {
    FreeWorld,
    Pe,
    Elf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Abi {
    FreeWorld64,
    MicrosoftX64,
    SystemVX64,
    Aapcs64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Architecture {
    X86_64,
    Arm64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionProfile {
    pub environment: Environment,
    pub image_format: ImageFormat,
    pub abi: Abi,
    pub architecture: Architecture,
}
