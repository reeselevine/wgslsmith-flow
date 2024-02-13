pub mod cli;

use clap::clap_derive::ArgEnum;
use serde::{Deserialize, Serialize};

use ast::types::{DataType, MemoryViewType, ScalarType};
use ast::{
    AccessMode, AssignmentLhs, AssignmentOp, AssignmentStatement, BinOp, BinOpExpr, Else, ExprNode,
    FnAttr, FnDecl, FnInput, FnInputAttr, ForLoopHeader, ForLoopInit, ForLoopUpdate, GlobalVarAttr,
    GlobalVarDecl, IfStatement, LetDeclStatement, Lit, Module, Postfix, PostfixExpr, ShaderStage,
    Statement, StorageClass, VarDeclStatement, VarExpr, VarQualifier, ForLoopStatement, FnCallExpr, BuiltinFn,
};

use rand::distributions::WeightedIndex;
use rand::prelude::{SliceRandom, StdRng};
use rand::{Rng, SeedableRng};
use rand_distr::Distribution;
use std::cmp;

#[derive(PartialEq, Eq, Copy, Clone, Debug)]
enum AccessType {
    ThreadSafe,
    ThreadUnsafe,
    ThreadRace,
    ConstantSafe,
    ConstantUnsafe,
    VarSafe,
    VarUnsafe,
    Literal,
}

#[derive(Serialize, Deserialize, Debug, ArgEnum, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum RaceValueStrategy {
    Even,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DataRaceInfo {
    pub safe: Vec<u32>,
    pub locs_per_thread: u32,
    pub safe_constants: Vec<u32>,
    pub constant_locs: u32,
    pub num_uninit_vars: u32,
    pub safe_vars: Vec<String>,
    pub race_val_strat: Option<RaceValueStrategy>,
}

pub struct Shaders {
    pub safe: Module,
    pub race: Module,
    pub info: DataRaceInfo,
}

pub struct GenOptions {
    pub seed: u64,
    pub workgroup_size: u32,
    pub racy_loc_pct: u32,
    pub racy_constant_loc_pct: u32,
    pub block_max_stmts: u32,
    pub block_max_nest_level: u32,
    pub else_chance: u32,
    pub max_loop_iter: u32,
    pub racy_var_pct: u32,
    pub num_lits: u32,
    pub stmts: u32,
    pub vars: u32,
    pub uninit_vars: u32,
    pub locs_per_thread: u32,
    pub constant_locs: u32,
    pub race_val_strat: Option<RaceValueStrategy>,
    pub oob_pct: u32,
}

pub fn gen(options: &GenOptions) -> Shaders {
    let mut rng = StdRng::seed_from_u64(options.seed);
    Generator::new(&mut rng, &options).gen_module()
}

pub struct Generator<'a> {
    rng: &'a mut StdRng,
    options: &'a GenOptions,
    safe_vars: Vec<String>,
    racy_vars: Vec<String>,
    uninit_vars: Vec<String>,
    lits: Vec<u32>,
    safe_offsets: Vec<u32>,
    racy_offsets: Vec<u32>,
    safe_constant_locs: Vec<u32>,
    racy_constant_locs: Vec<u32>,
    rhs_access_types: Vec<AccessType>,
    rhs_weights: WeightedIndex<i32>,
    safe_rhs_access_types: Vec<AccessType>,
    safe_rhs_weights: WeightedIndex<i32>,
    lhs_access_types: Vec<AccessType>,
    lhs_weights: WeightedIndex<i32>,
    racy_lhs_access_types: Vec<AccessType>,
    racy_lhs_weights: WeightedIndex<i32>,
}

const SAFE_ACCESS_TYPES: [AccessType; 4] = [
    AccessType::ConstantSafe,
    AccessType::Literal,
    AccessType::ThreadSafe,
    AccessType::VarSafe,
];

const RACY_ACCESS_TYPES: [AccessType; 4] = [
    AccessType::ConstantUnsafe,
    AccessType::ThreadUnsafe,
    AccessType::ThreadRace,
    AccessType::VarUnsafe,
];

pub struct StatementGenInfo {
    pub generated_statements: u32,
    pub statement: Statement,
    pub safe_statement: Option<Statement>,
}

impl<'a> Generator<'a> {
    pub fn new(rng: &'a mut StdRng, options: &'a GenOptions) -> Self {
        let mut lhs_access_types = vec![];
        let mut racy_lhs_access_types = vec![];
        let mut rhs_access_types = vec![AccessType::Literal];
        let mut safe_rhs_access_types = vec![AccessType::Literal];

        let mut lits = vec![];
        // Create literals used in the program
        for i in 0..options.num_lits {
            let val = match options.race_val_strat {
                Some(RaceValueStrategy::Even) => i * 2,
                None => i,
            };
            lits.push(val);
        }

        // The first N memory locations are divided into safe (read-only) and racy (read-write)
        let mut racy_constant_locs = vec![];
        let mut safe_constant_locs = vec![];
        for i in 0..options.constant_locs {
            if Self::rng_less_than(rng, options.racy_constant_loc_pct) {
                racy_constant_locs.push(i);
                Self::add_if_not_exists(&mut lhs_access_types, AccessType::ConstantUnsafe);
                Self::add_if_not_exists(&mut racy_lhs_access_types, AccessType::ConstantUnsafe);
                Self::add_if_not_exists(&mut rhs_access_types, AccessType::ConstantUnsafe);
            } else {
                safe_constant_locs.push(i);
                Self::add_if_not_exists(&mut rhs_access_types, AccessType::ConstantSafe);
                Self::add_if_not_exists(&mut safe_rhs_access_types, AccessType::ConstantSafe);
            }
        }

        // Each thread's memory locations are divided into safe (read-write only by this thread)
        // and racy (read-write by any thread)
        let mut racy_offsets = vec![];
        let mut safe_offsets = vec![];

        for i in 0..options.locs_per_thread {
            if Self::rng_less_than(rng, options.racy_loc_pct) {
                racy_offsets.push(i);
                Self::add_if_not_exists(&mut lhs_access_types, AccessType::ThreadUnsafe);
                Self::add_if_not_exists(&mut lhs_access_types, AccessType::ThreadRace);
                Self::add_if_not_exists(&mut racy_lhs_access_types, AccessType::ThreadUnsafe);
                Self::add_if_not_exists(&mut racy_lhs_access_types, AccessType::ThreadRace);
                Self::add_if_not_exists(&mut rhs_access_types, AccessType::ThreadUnsafe);
                Self::add_if_not_exists(&mut rhs_access_types, AccessType::ThreadRace);
            } else {
                safe_offsets.push(i);
                Self::add_if_not_exists(&mut lhs_access_types, AccessType::ThreadSafe);
                Self::add_if_not_exists(&mut rhs_access_types, AccessType::ThreadSafe);
                Self::add_if_not_exists(&mut safe_rhs_access_types, AccessType::ThreadSafe);
            }
        }

        // local variables are also divided into safe and racy
        let mut racy_vars = vec![];
        let mut safe_vars = vec![];
        for i in 0..options.vars {
            let name = format!("var_{i}");
            if Self::rng_less_than(rng, options.racy_var_pct) {
                racy_vars.push(name.to_owned());
                Self::add_if_not_exists(&mut lhs_access_types, AccessType::VarUnsafe);
                Self::add_if_not_exists(&mut racy_lhs_access_types, AccessType::VarUnsafe);
                Self::add_if_not_exists(&mut rhs_access_types, AccessType::VarUnsafe);
            } else {
                safe_vars.push(name.to_owned());
                Self::add_if_not_exists(&mut lhs_access_types, AccessType::VarSafe);
                Self::add_if_not_exists(&mut rhs_access_types, AccessType::VarSafe);
                Self::add_if_not_exists(&mut safe_rhs_access_types, AccessType::VarSafe);
            }
        }

        let mut uninit_vars = vec![];
        for i in 0..options.uninit_vars {
          let name = format!("uninit_var_{i}");
          uninit_vars.push(name.to_owned());
        }

        let mut lhs_weight_values = vec![];
        for access_type in &lhs_access_types {
            lhs_weight_values.push(Self::lhs_access_type_weight(access_type));
        }
        let lhs_weights =
            WeightedIndex::new(lhs_weight_values).unwrap_or(WeightedIndex::new(vec![1]).unwrap());

        let mut racy_lhs_weight_values = vec![];
        for access_type in &racy_lhs_access_types {
            racy_lhs_weight_values.push(Self::racy_lhs_access_type_weight(access_type));
        }
        let racy_lhs_weights = WeightedIndex::new(racy_lhs_weight_values)
            .unwrap_or(WeightedIndex::new(vec![1]).unwrap());

        let mut rhs_weight_values = vec![];
        for access_type in &rhs_access_types {
            rhs_weight_values.push(Self::rhs_access_type_weight(access_type));
        }
        let rhs_weights =
            WeightedIndex::new(rhs_weight_values).unwrap_or(WeightedIndex::new(vec![1]).unwrap());

        let mut safe_rhs_weight_values = vec![];
        for access_type in &safe_rhs_access_types {
            safe_rhs_weight_values.push(Self::safe_rhs_access_type_weight(access_type));
        }
        let safe_rhs_weights = WeightedIndex::new(safe_rhs_weight_values)
            .unwrap_or(WeightedIndex::new(vec![1]).unwrap());

        Generator {
            rng,
            options,
            safe_vars,
            racy_vars,
            uninit_vars,
            lits,
            safe_offsets,
            racy_offsets,
            safe_constant_locs,
            racy_constant_locs,
            rhs_access_types,
            rhs_weights,
            safe_rhs_access_types,
            safe_rhs_weights,
            lhs_access_types,
            lhs_weights,
            racy_lhs_access_types,
            racy_lhs_weights,
            //curr_loop_var: 0,
        }
    }

    fn add_if_not_exists<T: PartialEq>(vector: &mut Vec<T>, value: T) {
        if !vector.contains(&value) {
            vector.push(value);
        }
    }

    fn rng_less_than(rng: &mut StdRng, pct: u32) -> bool {
        return rng.gen_range(0..100) < pct;
    }

    fn lhs_access_type_weight(access_type: &AccessType) -> i32 {
        match access_type {
            AccessType::ThreadSafe => 17,
            AccessType::ThreadUnsafe => 17,
            AccessType::ThreadRace => 17,
            AccessType::ConstantUnsafe => 17,
            AccessType::VarSafe => 16,
            AccessType::VarUnsafe => 16,
            _ => 0,
        }
    }

    fn racy_lhs_access_type_weight(access_type: &AccessType) -> i32 {
        match access_type {
            AccessType::ThreadUnsafe => 25,
            AccessType::ThreadRace => 25,
            AccessType::ConstantUnsafe => 25,
            AccessType::VarUnsafe => 25,
            _ => 0,
        }
    }

    fn rhs_access_type_weight(access_type: &AccessType) -> i32 {
        match access_type {
            AccessType::ThreadSafe => 13,
            AccessType::ThreadUnsafe => 13,
            AccessType::ThreadRace => 13,
            AccessType::ConstantSafe => 13,
            AccessType::ConstantUnsafe => 12,
            AccessType::VarSafe => 12,
            AccessType::VarUnsafe => 12,
            AccessType::Literal => 12,
        }
    }

    fn safe_rhs_access_type_weight(access_type: &AccessType) -> i32 {
        match access_type {
            AccessType::ThreadSafe => 30,
            AccessType::ConstantSafe => 30,
            AccessType::VarSafe => 20,
            AccessType::Literal => 20,
            _ => 0,
        }
    }

    #[tracing::instrument(skip(self))]
    pub fn gen_module(&mut self) -> Shaders {
        let global_vars = vec![
          GlobalVarDecl {
            attrs: vec![GlobalVarAttr::Group(0), GlobalVarAttr::Binding(0)],
            qualifier: Some(VarQualifier {
                storage_class: StorageClass::Storage,
                access_mode: Some(AccessMode::ReadWrite),
            }),
            name: "mem".to_owned(),
            data_type: DataType::array(ScalarType::U32, None),
            initializer: None,
          },
          GlobalVarDecl {
            attrs: vec![GlobalVarAttr::Group(0), GlobalVarAttr::Binding(1)],
            qualifier: Some(VarQualifier {
                storage_class: StorageClass::Storage,
                access_mode: Some(AccessMode::ReadWrite),
            }),
            name: "uninit_vars".to_owned(),
            data_type: DataType::array(ScalarType::I32, None),
            initializer: None,
          },
        ];

        let mut block: Vec<Statement> = vec![];

        // total ids is the number of workgroups times the workgroup size
        block.push(
            LetDeclStatement::new(
                "total_ids",
                BinOpExpr::new(
                    BinOp::Times,
                    VarExpr::new("num_workgroups.x").into_node(DataType::from(ScalarType::U32)),
                    ExprNode::from(Lit::U32(self.options.workgroup_size)),
                ),
            )
            .into(),
        );

        for var in self.safe_vars.to_owned() {
            block.push(self.initialize_var(var));
        }

        for var in self.racy_vars.to_owned() {
            block.push(self.initialize_var(var));
        }

        for var in self.uninit_vars.to_owned() {
            block.push(self.uninitialize_var(var));
        }

        for (i, var) in self.uninit_vars.to_owned().iter().enumerate() {
          block.push(self.read_uninitialized_var(var.clone(), i as u32));
        }

        // Make a block to store statements that are safe
        let mut safe_block: Vec<Statement> = block.clone();

        let mut stmts_left: u32 = self.options.stmts;

        while stmts_left > 0 {
            let gen_info = self.gen_statement(stmts_left, 0, false);

            block.push(gen_info.statement.clone());
            match gen_info.safe_statement {
                Some(stmt) => {
                  safe_block.push(stmt.clone());
                }
                None => {}
            }
            stmts_left -= gen_info.generated_statements;
        }

        // We add a dummy statement at the end of the shader to ensure the safe shader uses memory at least once, avoiding
        // over-zealous compiler errors
        let dummy_access_type = if self.safe_offsets.is_empty() { AccessType::ThreadUnsafe } else { AccessType::ThreadSafe };
        let dummy_stmt = VarDeclStatement::new("var_dummy", Some(ScalarType::U32.into()), Some(self.gen_op(dummy_access_type)));
        block.push(dummy_stmt.clone().into());
        safe_block.push(dummy_stmt.into());

        let mut num_workgroups =
            FnInput::new("num_workgroups", DataType::Vector(3, ScalarType::U32));
        num_workgroups
            .attrs
            .push(FnInputAttr::Builtin("num_workgroups".to_string()));
        let mut global_invocation_id =
            FnInput::new("global_invocation_id", DataType::Vector(3, ScalarType::U32));
        global_invocation_id
            .attrs
            .push(FnInputAttr::Builtin("global_invocation_id".to_string()));

        let entrypoint = FnDecl {
            attrs: vec![
                FnAttr::Stage(ShaderStage::Compute),
                FnAttr::LitWorkgroupSize(self.options.workgroup_size),
            ],
            name: "main".to_owned(),
            inputs: vec![num_workgroups.clone(), global_invocation_id.clone()],
            output: None,
            body: block,
        };

        let safe_entrypoint = FnDecl {
            attrs: vec![
                FnAttr::Stage(ShaderStage::Compute),
                FnAttr::LitWorkgroupSize(self.options.workgroup_size),
            ],
            name: "main".to_owned(),
            inputs: vec![num_workgroups, global_invocation_id],
            output: None,
            body: safe_block,
        };

        let functions = vec![entrypoint];
        let safe_functions = vec![safe_entrypoint];

        Shaders {
            safe: Module {
                structs: vec![],
                consts: vec![],
                vars: global_vars.clone(),
                functions: safe_functions,
            },
            race: Module {
                structs: vec![],
                consts: vec![],
                vars: global_vars.clone(),
                functions,
            },
            info: DataRaceInfo {
                safe: self.safe_offsets.clone(),
                locs_per_thread: self.options.locs_per_thread,
                safe_constants: self.safe_constant_locs.clone(),
                constant_locs: self.options.constant_locs,
                num_uninit_vars: self.options.uninit_vars,
                safe_vars: self.safe_vars.clone(),
                race_val_strat: self.options.race_val_strat,
            },
        }
    }

    fn initialize_var(&mut self, name: String) -> Statement {
        let ty = ScalarType::U32;
        let val = match self.options.race_val_strat {
            Some(RaceValueStrategy::Even) => 2,
            None => 1,
        };
        VarDeclStatement::new(name, Some(ty.into()), Some(ExprNode::from(Lit::U32(val)))).into()
    }

    fn uninitialize_var(&mut self, name: String) -> Statement {
        let ty = ScalarType::I32;
        VarDeclStatement::new(name, Some(ty.into()), None).into()
    }

    fn read_uninitialized_var(&mut self, name: String, offset: u32) -> Statement {
      let base_id = BinOpExpr::new(
        BinOp::Times,
      VarExpr::new("global_invocation_id.x")
              .into_node(DataType::from(ScalarType::U32)),
                  ExprNode::from(Lit::U32(self.options.uninit_vars)));
      let store_id = BinOpExpr::new(
        BinOp::Plus,
        base_id,
        ExprNode::from(Lit::U32(offset)));

      AssignmentStatement::new(
        AssignmentLhs::array_index(
          "uninit_vars",
          DataType::Ref(MemoryViewType::new(
            DataType::array(ScalarType::U32, None),
                    StorageClass::Storage)),
          store_id.into()),
        AssignmentOp::Simple,
    VarExpr::new(name).into_node(DataType::from(ScalarType::I32))).into()
    }


    fn constant_expr(&mut self, choices: Vec<u32>) -> ExprNode {
        ExprNode::from(Lit::U32(choices.choose(self.rng).unwrap().to_owned()))
    }

    fn gen_mem_idx(&mut self, access_type: AccessType) -> ExprNode {
        let offset = match access_type {
            AccessType::ThreadSafe => self.safe_offsets.choose(self.rng).unwrap().to_owned(),
            AccessType::ThreadUnsafe | AccessType::ThreadRace => {
                self.racy_offsets.choose(self.rng).unwrap().to_owned()
            }
            _ => 0, //unused
        };
        match access_type {
            AccessType::ConstantSafe => {
                let choices = self.safe_constant_locs.to_owned();
                self.constant_expr(choices)
            }
            AccessType::ConstantUnsafe => {
                let choices = self.racy_constant_locs.to_owned();
                self.constant_expr(choices)
            }
            AccessType::ThreadSafe | AccessType::ThreadUnsafe => {
                let base_id = BinOpExpr::new(
                    BinOp::Times,
                    VarExpr::new("global_invocation_id.x")
                        .into_node(DataType::from(ScalarType::U32)),
                    ExprNode::from(Lit::U32(self.options.locs_per_thread)),
                );
                BinOpExpr::new(
                    BinOp::Plus,
                    base_id,
                    ExprNode::from(Lit::U32(offset + self.options.constant_locs)),
                )
                .into()
            }
            AccessType::ThreadRace => {
                let new_id = BinOpExpr::new(
                    BinOp::Plus,
                    VarExpr::new("global_invocation_id.x")
                        .into_node(DataType::from(ScalarType::U32)),
                    ExprNode::from(Lit::U32(self.rng.gen_range(1..1024))),
                ); //TODO: magic number

                // some % of the time we let the write potentially go out of bounds, to test clamping
                let mod_id = if self.rng.gen_range(0..100) < self.options.oob_pct {
                    new_id
                } else {
                    BinOpExpr::new(
                        BinOp::Mod,
                        new_id,
                        VarExpr::new("total_ids").into_node(DataType::from(ScalarType::U32)),
                    )
                };

                let base_id = BinOpExpr::new(
                    BinOp::Times,
                    mod_id,
                    ExprNode::from(Lit::U32(self.options.locs_per_thread)),
                );
                BinOpExpr::new(
                    BinOp::Plus,
                    base_id,
                    ExprNode::from(Lit::U32(offset + self.options.constant_locs)),
                )
                .into()
            }
            _ => unreachable!(),
        }
    }

    fn gen_mem_access(&mut self, access_type: AccessType) -> ExprNode {
        let index = Postfix::index(self.gen_mem_idx(access_type));
        let arr_expr = VarExpr::new("mem").into_node(DataType::Ref(MemoryViewType::new(
            DataType::array(ScalarType::U32, None),
            StorageClass::Storage,
        )));
        PostfixExpr::new(arr_expr, index).into()
    }

    fn var_expr(&mut self, choices: Vec<String>) -> ExprNode {
        VarExpr::new(choices.choose(self.rng).unwrap()).into_node(DataType::from(ScalarType::U32))
    }

    fn gen_op(&mut self, access_type: AccessType) -> ExprNode {
        match access_type {
            AccessType::Literal => {
                ExprNode::from(Lit::U32(self.lits.choose(self.rng).unwrap().to_owned()))
            }
            AccessType::VarSafe => {
                let choices = self.safe_vars.to_owned();
                self.var_expr(choices)
            }
            AccessType::VarUnsafe => {
                let choices = self.racy_vars.to_owned();
                self.var_expr(choices)
            }
            _ => self.gen_mem_access(access_type),
        }
    }

    fn operand_access_ty(&mut self, access_type: &AccessType) -> AccessType {
        if SAFE_ACCESS_TYPES.contains(access_type) {
            self.safe_rhs_access_types[self.safe_rhs_weights.sample(self.rng)]
        } else {
            self.rhs_access_types[self.rhs_weights.sample(self.rng)]
        }
    }

    fn gen_expr(&mut self, access_type: &AccessType) -> ExprNode {
        let operand_weight = self.rng.gen_range(0..100);
        // 30% of the time we generate a single operand
        if operand_weight < 30 {
            let uni_type = self.operand_access_ty(access_type);
            self.gen_op(uni_type)
        } else {
            let left_access_type = self.operand_access_ty(access_type);
            let right_access_type = self.operand_access_ty(access_type);
            let expr = BinOpExpr::new(
                BinOp::Plus,
                self.gen_op(left_access_type),
                self.gen_op(right_access_type),
            );
            // 60% of the time we generate two operands
            if operand_weight < 90 {
                expr.into()
            // 10% of the time we generate three operands
            } else {
                let third_access_type = self.operand_access_ty(access_type);
                BinOpExpr::new(BinOp::Plus, expr, self.gen_op(third_access_type)).into()
            }
        }
    }

    // Generates two statements one that is a racy variant (the 1st) and the other that is safe (the 2nd)
    // Rates (TODO: Parameterize these)
    // 80% to generate an assignment
    // 10% to generate an if
    // 10% to generate a loop

    fn gen_statement(
        &mut self,
        stmts_left: u32,
        nest_level: u32,
        racy_block: bool,
    ) -> StatementGenInfo {
        // Randomly pick what kind of statement to generate, then return the statement back
        let decider = self.rng.gen_range(0..100);
        if decider < 80 || nest_level == self.options.block_max_nest_level || stmts_left < 2 {
            // Gen assign
            return self.gen_assign(racy_block);
        } else if decider < 90 {
            // Gen if
            return self.gen_if(stmts_left, nest_level + 1, racy_block);
        } else {
            // Gen loop
            return self.gen_for(stmts_left, nest_level + 1, racy_block);
        }
    }

    fn gen_assign(&mut self, racy_block: bool) -> StatementGenInfo {
        let lhs_access_type = if racy_block {
            self.racy_lhs_access_types[self.racy_lhs_weights.sample(self.rng)]
        } else {
            self.lhs_access_types[self.lhs_weights.sample(self.rng)]
        };
        let expr = self.gen_expr(&lhs_access_type.clone());
        let stmt: Statement = match lhs_access_type {
            AccessType::VarSafe => {
                let var = self.safe_vars.choose(self.rng).unwrap();
                AssignmentStatement::new(
                    AssignmentLhs::name(var, DataType::from(ScalarType::U32)),
                    AssignmentOp::Simple,
                    expr,
                )
                .into()
            }
            AccessType::VarUnsafe => {
                let var = self.racy_vars.choose(self.rng).unwrap();
                AssignmentStatement::new(
                    AssignmentLhs::name(var, DataType::from(ScalarType::U32)),
                    AssignmentOp::Simple,
                    expr,
                )
                .into()
            }
            _ => AssignmentStatement::new(
                AssignmentLhs::array_index(
                    "mem",
                    DataType::Ref(MemoryViewType::new(
                        DataType::array(ScalarType::U32, None),
                        StorageClass::Storage,
                    )),
                    self.gen_mem_idx(lhs_access_type),
                ),
                AssignmentOp::Simple,
                expr,
            )
            .into(),
        };
        let safe_stmt_opt = if SAFE_ACCESS_TYPES.contains(&lhs_access_type) {
            Some(stmt.clone())
        } else {
            None
        };
        StatementGenInfo {
            generated_statements: 1,
            statement: stmt,
            safe_statement: safe_stmt_opt,
        }
    }

    fn gen_comp_expr(
        &mut self,
        first_access_type: AccessType,
        second_access_type: AccessType,
    ) -> ExprNode {
        let comp_weight = self.rng.gen_range(0..100);

        // Comparison will be array[index] (< or >) literal value
        // 50% of the time generate a >
        if comp_weight < 50 {
            BinOpExpr::new(
                BinOp::Greater,
                self.gen_op(first_access_type),
                self.gen_op(second_access_type),
            )
            .into()
        } else {
            BinOpExpr::new(
                BinOp::Less,
                self.gen_op(first_access_type),
                self.gen_op(second_access_type),
            )
            .into()
        }
    }

    fn gen_if(&mut self, stmts_left: u32, nest_level: u32, racy_block: bool) -> StatementGenInfo {
        // Access types in the if comparison can be racy or non-racy, since no writes occur
        let first_if_access_type = self.rhs_access_types[self.rhs_weights.sample(self.rng)];

        let cur_block_racy = racy_block || RACY_ACCESS_TYPES.contains(&first_if_access_type);

        // to give a 50% chance of a block being non-racy, we check if we are still in a safe block
        let second_if_access_type = if cur_block_racy {
            self.rhs_access_types[self.rhs_weights.sample(self.rng)]
        } else {
            self.safe_rhs_access_types[self.safe_rhs_weights.sample(self.rng)]
        };

        let comp = self.gen_comp_expr(first_if_access_type, second_if_access_type);

        let mut race_if_stmts: Vec<Statement> = Vec::new();
        let mut safe_if_stmts: Vec<Statement> = Vec::new();

        // the block contains the minimum of a range of statements or the number of statements left (minus the one statement
        // used for the block condition)
        let mut num_statements = cmp::min(
            self.rng.gen_range(1..self.options.block_max_stmts),
            stmts_left - 1,
        );
        let mut generated_statements = num_statements + 1;

        while num_statements > 0 {
            let gen_info = self.gen_statement(num_statements, nest_level, cur_block_racy);
            race_if_stmts.push(gen_info.statement);
            match gen_info.safe_statement {
                Some(stmt) => safe_if_stmts.push(stmt),
                None => {}
            };
            num_statements -= gen_info.generated_statements;
        }

        let race_if_stmt = IfStatement::new(comp.clone(), race_if_stmts.clone());
        let safe_if_stmt = IfStatement::new(comp.clone(), safe_if_stmts.clone());

        // Have a chance that the if has an else block, if there are enough remaining statements to do so
        let remaining_stmts = stmts_left - generated_statements;
        if remaining_stmts > 0 && self.rng.gen_range(0..100) <= self.options.else_chance {
            let mut race_else_stmts: Vec<Statement> = Vec::new();
            let mut safe_else_stmts: Vec<Statement> = Vec::new();
            let mut num_else_statements = cmp::min(
                self.rng.gen_range(1..self.options.block_max_stmts),
                remaining_stmts,
            );
            generated_statements += num_else_statements;

            while num_else_statements > 0 {
                let gen_info = self.gen_statement(num_else_statements, nest_level, cur_block_racy);
                race_else_stmts.push(gen_info.statement);
                match gen_info.safe_statement {
                    Some(stmt) => safe_else_stmts.push(stmt),
                    None => {}
                };
                num_else_statements -= gen_info.generated_statements;
            }

            let race_if_else = race_if_stmt.with_else(Else::Else(race_else_stmts));
            let safe_if_else = safe_if_stmt.with_else(Else::Else(safe_else_stmts));
            let safe_stmt_opt = if cur_block_racy {
                None
            } else {
                Some(Statement::If(safe_if_else))
            };
            StatementGenInfo {
                generated_statements: generated_statements, // statements in each block plus 1 for the if condition
                statement: Statement::If(race_if_else),
                safe_statement: safe_stmt_opt,
            }
        } else {
            let safe_stmt_opt = if cur_block_racy {
                None
            } else {
                Some(Statement::If(safe_if_stmt))
            };
            StatementGenInfo {
                generated_statements: generated_statements, // statements in the if block plus 1 for the if condition
                statement: Statement::If(race_if_stmt),
                safe_statement: safe_stmt_opt,
            }
        }
    }

    fn gen_for_header(&mut self, access_type: AccessType, nest_level: u32) -> ForLoopHeader {
        let ident = format!("i_{}", nest_level);
        let expr = self.gen_op(access_type);
        let args = vec![expr, Lit::U32(self.options.max_loop_iter).into()];
        let min_expr = FnCallExpr::new(BuiltinFn::Min.as_ref(), args);
        let init = ForLoopInit::VarDecl(VarDeclStatement::new(
            &ident,
            Some(DataType::from(ScalarType::U32)),
            Some(min_expr.into_node(DataType::from(ScalarType::U32)))
        ));
        let for_cond = BinOpExpr::new(
            BinOp::Greater,
            VarExpr::new(&ident).into_node(DataType::from(ScalarType::U32)),
            Lit::U32(0),
        );
        let for_update = AssignmentStatement::new(
            AssignmentLhs::name(&ident, DataType::from(ScalarType::U32)),
            AssignmentOp::Simple,
            BinOpExpr::new(
                BinOp::Minus,
                VarExpr::new(ident).into_node(DataType::from(ScalarType::U32)),
                Lit::U32(1),
            ),
        );
        ForLoopHeader {
            init: Some(init),
            condition: Some(for_cond.into()),
            update: Some(ForLoopUpdate::Assignment(for_update)),
        }
    }

    fn gen_for(&mut self, stmts_left: u32, nest_level: u32, racy_block: bool) -> StatementGenInfo {
        // Access types in the loop initializer can be racy or non-racy, since no writes occur
        let loop_initializer = self.rhs_access_types[self.rhs_weights.sample(self.rng)];

        let cur_block_racy = racy_block || RACY_ACCESS_TYPES.contains(&loop_initializer);

        let for_loop_header = self.gen_for_header(loop_initializer, nest_level);

        let mut race_for_stmts: Vec<Statement> = Vec::new();
        let mut safe_for_stmts: Vec<Statement> = Vec::new();

        // the block contains the minimum of a range of statements or the number of statements left (minus the one statement
        // used for the for loop initialization)
        let mut num_statements = cmp::min(
            self.rng.gen_range(1..self.options.block_max_stmts),
            stmts_left - 1,
        );
        let generated_statements = num_statements + 1;

        while num_statements > 0 {
            let gen_info = self.gen_statement(num_statements, nest_level, cur_block_racy);
            race_for_stmts.push(gen_info.statement);
            match gen_info.safe_statement {
                Some(stmt) => safe_for_stmts.push(stmt),
                None => {}
            };
            num_statements -= gen_info.generated_statements;
        }

        let race_for_stmt = ForLoopStatement::new(for_loop_header.clone(), race_for_stmts);
        let safe_for_stmt = ForLoopStatement::new(for_loop_header, safe_for_stmts);

        let safe_stmt_opt = if cur_block_racy {
            None
        } else {
            Some(Statement::ForLoop(safe_for_stmt))
        };
        StatementGenInfo {
            generated_statements: generated_statements, // statements in the block plus 1 for the initializer
            statement: Statement::ForLoop(race_for_stmt),
            safe_statement: safe_stmt_opt,
        }
    }
}
