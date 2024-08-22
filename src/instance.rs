use log::{debug, error, info, trace, warn};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap, path::Path, sync::{Arc, Mutex}
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    process,
    sync::{broadcast, mpsc, oneshot},
    task,
};

static BROKER_SENDER: Lazy<Mutex<Option<mpsc::Sender<IOperation>>>> =
    Lazy::new(|| Mutex::new(None));

fn default_false() -> bool {
    false
}

fn default_stop() -> String {
    "stop".to_string()
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub enum InstanceStatus {
    Running,
    Stopped,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InstanceInfo {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) app: String,
    pub(crate) parameter: Vec<String>,
    pub(crate) auto_run: bool,

    #[serde(default = "default_false")]
    pub(crate) auto_restart: bool,

    #[serde(default = "default_stop")]
    pub(crate) stop_command: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RespInstanceInfo {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) app: String,
    pub(crate) parameter: Vec<String>,
    pub(crate) auto_run: bool,

    pub(crate) status: InstanceStatus,
}

impl RespInstanceInfo {
    fn new(inst_info: InstanceInfo, status: InstanceStatus) -> Self {
        RespInstanceInfo {
            name: inst_info.name,
            path: inst_info.path,
            app: inst_info.app,
            parameter: inst_info.parameter,
            auto_run: inst_info.auto_run,
            status,
        }
    }
}

pub enum OkResult {
    Receiver(broadcast::Receiver<String>),
    Message(String),
    None,
}

pub type InstanceOperationResult = Result<OkResult, String>;

pub struct Instance {
    instance_info: InstanceInfo,
    process: Option<process::Child>,
    sender: broadcast::Sender<String>,
    last_log: Arc<Mutex<String>>,
    logging_handler: Option<task::JoinHandle<()>>,
    manually_stopped: bool,
    stdin: Option<process::ChildStdin>,
}

impl Instance {
    fn new(instance_info: InstanceInfo, sender: broadcast::Sender<String>) -> Self {
        Instance {
            instance_info,
            process: None,
            sender,
            last_log: Arc::new(Mutex::new(String::new())),
            logging_handler: None,
            manually_stopped: true,
            stdin: None,
        }
    }

    fn start(&mut self) -> InstanceOperationResult {
        let mut cmd = process::Command::new(&self.instance_info.app);
        cmd.args(&self.instance_info.parameter);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.stdin(std::process::Stdio::piped());
        cmd.current_dir(&self.instance_info.path);

        match &self.logging_handler {
            Some(task) => {
                task.abort();
            }
            None => {}
        };
        self.logging_handler = None;

        match cmd.spawn() {
            Ok(mut p) => {
                let mut output = p.stdout.take().unwrap();
                self.stdin = Some(p.stdin.take().unwrap());
                let sender = self.sender.clone();
                let last_log = self.last_log.clone(); // Clone the Arc

                self.logging_handler = Some(tokio::spawn(async move {
                    output_printer(&mut output, sender, last_log).await;
                }));

                self.process = Some(p);
                self.manually_stopped = false;
                Ok(OkResult::None)
            }
            Err(e) => {
                error!("[{}]{}", &self.instance_info.name, e);
                Err(e.to_string())
            }
        }
        
    }

    async fn execute(&mut self, command: &String) -> InstanceOperationResult {
        if let Some(stdin) = self.stdin.as_mut() {
            if let Err(e) = stdin.write_all(format!("{}\n", command).as_bytes()).await {
                return Err(format!("Failed to write to stdin: {}", e));
            }
        } else {
            return Err("Instance child process is not available".to_string());
        }
        Ok(OkResult::None)
    }

    async fn halt(&mut self) -> InstanceOperationResult {
        if let Some(process) = self.process.as_mut() {
            self.manually_stopped = true;
            if let Err(e) = process.kill().await {
                return Err(format!("Failed to kill process: {}", e));
            }
        } else {
            return Err("Instance child process is not available".to_string());
        }
        Ok(OkResult::None)
    }

    fn status(&self) -> InstanceStatus {
        match self.process {
            Some(ref p) if p.id().is_some() => InstanceStatus::Running,
            _ => InstanceStatus::Stopped,
        }
    }

    async fn get_log(&self) -> String {
        let last_log = self.last_log.lock();
        last_log.unwrap().clone()
    }
}

#[derive(Debug)]
pub enum IOperationTypes {
    Create(InstanceInfo),
    Start(String),
    Stop(String),
    Halt(String),
    Remove(String),
    Execute(String, String),
    StdoutRead(String),
    StdoutGetReader(String),
    SaveConfiguration,
    ListInstances,
    Update(String, InstanceInfo),
    Shutdown,
    KeepInstanceRunning,
}

pub struct IOperation {
    pub(crate) otype: IOperationTypes,
    pub(crate) result: Option<oneshot::Sender<InstanceOperationResult>>,
}

async fn instance_save(instance_map: &HashMap<String, Instance>) -> InstanceOperationResult {
    let mut result: Vec<InstanceInfo> = Vec::new();
    for value in instance_map.values() {
        let value_info = value.instance_info.clone();
        result.push(value_info);
    }

    let file_result = File::create("instances.json").await;
    match file_result {
        Ok(mut file) => {
            let write_result = file
                .write_all(serde_json::to_string(&result).unwrap().as_bytes())
                .await;
            match write_result {
                Ok(_) => Ok(OkResult::None),
                Err(e) => Err(format!("Failed to write to file: {}", e)),
            }
        }
        Err(e) => Err(format!("Failed to create file: {}", e)),
    }
}

async fn instance_load(instance_map: &mut HashMap<String, Instance>) -> InstanceOperationResult {
    let mut file = match File::open("instances.json").await {
        Ok(file) => file,
        Err(e) => return Err(format!("Failed to open file: {}", e)),
    };

    let mut contents = String::new();
    if let Err(e) = file.read_to_string(&mut contents).await {
        return Err(format!("Failed to read file: {}", e));
    }

    debug!("Got configuration string.");

    let instances: Vec<InstanceInfo> = match serde_json::from_str(&contents) {
        Ok(instances) => instances,
        Err(e) => return Err(format!("Failed to parse JSON: {}", e)),
    };

    debug!("Starting auto_run instances");
    for instance_info in instances {
        let (sender, _) = broadcast::channel(32);
        let mut instance = Instance::new(instance_info.clone(), sender);
        if instance.instance_info.auto_run {
            let _ = instance.start();
        }
        instance_map.insert(instance_info.name.clone(), instance);
    }

    Ok(OkResult::None)
}

async fn instance_update(
    instance_map: &mut HashMap<String, Instance>,
    name: String,
    new_info: InstanceInfo,
) -> InstanceOperationResult {
    match instance_map.get_mut(&name) {
        None => Err("Invalid instance name".to_string()),
        Some(instance) => {
            if instance.status() == InstanceStatus::Running {
                Err("Cannot update running instance".to_string())
            } else {
                instance.instance_info = new_info;
                Ok(OkResult::None)
            }
        }
    }
}

fn instance_create(
    instance_map: &mut HashMap<String, Instance>,
    instance_info: InstanceInfo,
    sender: broadcast::Sender<String>,
) -> InstanceOperationResult {
    if instance_map.contains_key(&instance_info.name) {
        return InstanceOperationResult::Err("Instance exist".to_string());
    }
    if !Path::new(&instance_info.path).is_dir() {
        return InstanceOperationResult::Err("Path not exists".to_string());
    }
    instance_map.insert(
        instance_info.name.clone(),
        Instance::new(instance_info, sender),
    );
    return InstanceOperationResult::Ok(OkResult::None);
}

fn instance_start(
    instance_map: &mut HashMap<String, Instance>,
    name: String,
) -> InstanceOperationResult {
    match instance_map.get_mut(&name) {
        None => Err("Invalid instance name".to_string()),
        Some(instance) => {
            if instance.status() == InstanceStatus::Running {
                Err("Instance is already running".to_string())
            } else {
                instance.start()
            }
        }
    }
}

async fn instance_execute(
    instance_map: &mut HashMap<String, Instance>,
    name: String,
    command: String,
) -> InstanceOperationResult {
    match instance_map.get_mut(&name) {
        None => Err("Invalid instance name".to_string()),
        Some(instance) => {
            if instance.status() == InstanceStatus::Stopped {
                Err("Instance is stopped".to_string())
            } else {
                instance.execute(&command).await
            }
        }
    }
}

async fn instance_stop(
    instance_map: &mut HashMap<String, Instance>,
    name: String,
) -> InstanceOperationResult {
    match instance_map.get_mut(&name) {
        None => Err("Invalid instance name".to_string()),
        Some(instance) => {
            if instance.status() == InstanceStatus::Stopped {
                Err("Instance is already stopped".to_string())
            } else {
                instance.manually_stopped = true;

                let cmd = instance.instance_info.stop_command.clone();
                instance.execute(&cmd).await
            }
        }
    }
}

fn instance_try_remove(
    instance_map: &mut HashMap<String, Instance>,
    name: String,
) -> InstanceOperationResult {
    match instance_map.get_mut(&name) {
        None => Err("Invalid instance name".to_string()),
        Some(instance) => {
            let name = instance.instance_info.name.clone();
            if instance.status() == InstanceStatus::Running {
                Err("Instance is still running, please stop it first!".to_string())
            } else {
                instance_map.remove(&name);
                Ok(OkResult::None)
            }
        }
    }
}

async fn output_printer(
    stdout: &mut process::ChildStdout,
    sender: broadcast::Sender<String>,
    last_log: Arc<Mutex<String>>,
) {
    let mut buf = [0; 1024];

    loop {
        match stdout.read(&mut buf).await {
            Ok(0) => {
                continue;
            }
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]);
                {
                    let mut last_log = last_log.lock().unwrap();
                    last_log.push_str(&chunk);
                    if last_log.len() > 10240 {
                        // Ensure we don't cut off in the middle of a UTF-8 character
                        let excess = last_log.len() - 10240;
                        let valid_up_to = last_log
                            .char_indices()
                            .rev()
                            .find(|&(i, _)| i <= excess)
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        last_log.drain(..valid_up_to);
                    }
                }
                let _ = sender.send(chunk.to_string());
            }
            Err(e) => {
                error!("Output printer encountered an error: {}", e);
                break;
            }
        }
    }
}

fn auto_resume(
    instance_map: &mut HashMap<String, Instance>,
) {
    for (name, instance) in instance_map.iter_mut() {
        if (instance.instance_info.auto_restart) && (instance.status() == InstanceStatus::Stopped) && (!instance.manually_stopped) {
            info!("Accidently stopped instance detected, restarting: {}", name);
            let _ = instance.start();
        }
    }
}

pub async fn instance_broker(
    mut operation_pipe: mpsc::Receiver<IOperation>,
    sender: mpsc::Sender<IOperation>,
) {
    debug!("Instance broker started");

    {
        let mut g = BROKER_SENDER.lock().unwrap();
        *g = Some(sender);
    }

    let mut instance_map: HashMap<String, Instance> = HashMap::new();
    debug!("Initializing instance list");
    {
        let result = instance_load(&mut instance_map).await;
        match result {
            Ok(_) => {
                info!("Instance configuration loaded");
            }
            Err(m) => {
                warn!("Instance configuration loading failed: {}", m);
            }
        };
    }

    debug!("Awaiting instructions");

    while let Some(operation) = operation_pipe.recv().await {
        trace!("Got instruction: {:?}", operation.otype);
        match operation.otype {
            IOperationTypes::Create(instance_info) => {
                let (sender, _) = broadcast::channel::<String>(32);

                let _ = operation.result.unwrap().send(instance_create(
                    &mut instance_map,
                    instance_info,
                    sender,
                ));
            }
            IOperationTypes::Start(name) => {
                let _ = operation
                    .result
                    .unwrap()
                    .send(instance_start(&mut instance_map, name));
            }
            IOperationTypes::Stop(name) => {
                let result = instance_stop(&mut instance_map, name).await;
                let _ = operation.result.unwrap().send(result);
            }
            IOperationTypes::Halt(name) => {
                let _ = operation.result.unwrap().send(match instance_map.get_mut(&name) {
                    Some(instance) => instance.halt().await,
                    None => Err("Invalid instance name".to_string()),
                });
            }
            IOperationTypes::Remove(name) => {
                let _ = operation
                    .result
                    .unwrap()
                    .send(instance_try_remove(&mut instance_map, name));
            }
            IOperationTypes::Execute(name, command) => {
                let _ = operation
                    .result
                    .unwrap()
                    .send(instance_execute(&mut instance_map, name, command).await);
            }
            IOperationTypes::StdoutRead(name) => {
                let _ = operation.result.unwrap().send(match instance_map.get_mut(&name) {
                    Some(instance) => Ok(OkResult::Message(instance.get_log().await)),
                    None => Err("Invalid instance name".to_string()),
                });
            }
            IOperationTypes::StdoutGetReader(name) => {
                let _ = operation.result.unwrap().send(match instance_map.get_mut(&name) {
                    Some(instance) => Ok(OkResult::Receiver(instance.sender.subscribe())),
                    None => Err("Invalid instance name".to_string()),
                });
            }
            IOperationTypes::SaveConfiguration => {
                let _ = operation.result.unwrap().send(instance_save(&instance_map).await);
            }
            IOperationTypes::ListInstances => {
                let instances: Vec<RespInstanceInfo> = instance_map
                    .values()
                    .map(|instance| {
                        RespInstanceInfo::new(instance.instance_info.clone(), instance.status())
                    })
                    .collect();
                let _ = operation.result.unwrap().send(Ok(OkResult::Message(
                    serde_json::to_string(&instances).unwrap(),
                )));
            }
            IOperationTypes::Update(name, new_info) => {
                let result = instance_update(&mut instance_map, name, new_info).await;
                let _ = operation.result.unwrap().send(result);
            }
            IOperationTypes::Shutdown => {
                for (_, instance) in instance_map.iter_mut() {
                    let _ = instance.halt().await;
                    break;
                }
            }
            IOperationTypes::KeepInstanceRunning => {
                auto_resume(&mut instance_map);
            }
        }
    }
}
